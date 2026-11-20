//! Accounts, sessions and password hashing.

use std::panic::AssertUnwindSafe;
use std::sync::{Arc, Mutex, PoisonError, mpsc};

use argon2::{Algorithm, Argon2, Params, PasswordHasher, PasswordVerifier, Version};
use http::{HeaderMap, StatusCode, header};
use rusqlite::{Connection, TransactionBehavior};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::App;
use crate::config::{Config, SecureCookie};
use crate::db::{Db, opt};
use crate::http::{Ctx, HResult, JSON_LIMIT, Req, is_https, json_response, put_header, read_json};
use crate::ratelimit::Key;
use crate::util::{
    AppError, WEEK_MS, hash_token, is_hidden_char, is_token, new_token, now_ms, random_password,
};

/// Session cookie over plain HTTP.
pub const SESSION_COOKIE: &str = "od_sid";
/// Session cookie when it is marked Secure. Browsers accept a `__Host-`
/// cookie only from this very host (Secure, Path=/, no Domain), so a page on
/// a sibling subdomain cannot plant its own session on our users.
pub const SECURE_SESSION_COOKIE: &str = "__Host-od_sid";
const SESSION_MAX_AGE_SECS: i64 = 7 * 24 * 3600;
/// Oldest sessions beyond this many per account are dropped on login.
pub const MAX_SESSIONS_PER_ACCOUNT: i64 = 32;
pub const MIN_ADMIN_PASSWORD: usize = 8;
pub const MAX_PASSWORD: usize = 128;

// ---------------------------------------------------------------------------
// Password hashing (argon2id m=19456 KiB, t=2, p=1), at most two at a time.
// ---------------------------------------------------------------------------

fn argon() -> Result<Argon2<'static>, AppError> {
    let params =
        Params::new(19_456, 2, 1, None).map_err(|e| AppError::internal("argon2 params", e))?;
    Ok(Argon2::new(Algorithm::Argon2id, Version::V0x13, params))
}

pub fn hash_blocking(password: &str) -> Result<String, AppError> {
    let hash = argon()?
        .hash_password(password.as_bytes())
        .map_err(|e| AppError::internal("argon2 hash", e))?;
    Ok(hash.to_string())
}

pub fn verify_blocking(password: &str, hash: &str) -> bool {
    argon().is_ok_and(|a| a.verify_password(password.as_bytes(), hash).is_ok())
}

type Job = Box<dyn FnOnce() + Send>;

/// Threads that run every argon2 hash and verification.
///
/// Each run allocates a fresh 19 MiB block buffer. On glibc that buffer is
/// served from the malloc arena of whichever thread runs it and stays
/// resident after it is freed (the first free raises the mmap threshold), so
/// spreading the work over the many threads of the blocking pool would keep
/// one such buffer per arena, up to about 8 per core. Running it on two
/// long lived threads caps that at two buffers. musl (the shipped image)
/// returns the memory either way.
pub struct HashPool {
    jobs: mpsc::Sender<Job>,
}

/// Concurrent hashes; also the number of hashing threads.
pub const HASH_THREADS: usize = 2;

impl HashPool {
    pub fn new() -> Result<HashPool, String> {
        let (jobs, rx) = mpsc::channel::<Job>();
        let rx = Arc::new(Mutex::new(rx));
        for i in 0..HASH_THREADS {
            let rx = Arc::clone(&rx);
            std::thread::Builder::new()
                .name(format!("argon2-{i}"))
                .spawn(move || {
                    loop {
                        let job = rx.lock().unwrap_or_else(PoisonError::into_inner).recv();
                        match job {
                            // a panic must not take the thread with it
                            Ok(job) => drop(std::panic::catch_unwind(AssertUnwindSafe(job))),
                            Err(_) => break, // the server is gone
                        }
                    }
                })
                .map_err(|e| format!("cannot start hashing thread: {e}"))?;
        }
        Ok(HashPool { jobs })
    }

    /// Runs `f` on a hashing thread. Callers hold a `hash_sem` permit, so
    /// at most `HASH_THREADS` jobs are ever queued.
    async fn run<R: Send + 'static>(
        &self,
        f: impl FnOnce() -> R + Send + 'static,
    ) -> Result<R, AppError> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.jobs
            .send(Box::new(move || {
                let _ = tx.send(f());
            }))
            .map_err(|_| AppError::internal("hash pool", "stopped"))?;
        rx.await
            .map_err(|_| AppError::internal("hash pool", "job failed"))
    }
}

pub async fn hash_password(app: &App, password: String) -> Result<String, AppError> {
    let permit = Arc::clone(&app.hash_sem)
        .acquire_owned()
        .await
        .map_err(|e| AppError::internal("hash semaphore", e))?;
    app.hasher
        .run(move || {
            let r = hash_blocking(&password);
            drop(permit);
            r
        })
        .await?
}

pub async fn verify_password(app: &App, password: String, hash: String) -> Result<bool, AppError> {
    let permit = Arc::clone(&app.hash_sem)
        .acquire_owned()
        .await
        .map_err(|e| AppError::internal("hash semaphore", e))?;
    app.hasher
        .run(move || {
            let ok = verify_blocking(&password, &hash);
            drop(permit);
            ok
        })
        .await
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

/// Trimmed username of 1..=32 characters without whitespace or invisible
/// characters.
pub fn valid_username(raw: &str) -> Result<String, AppError> {
    let u = raw.trim();
    let n = u.chars().count();
    if n == 0 || n > 32 || u.chars().any(|c| c.is_whitespace() || is_hidden_char(c)) {
        return Err(AppError::bad_field("username"));
    }
    Ok(u.to_owned())
}

pub fn valid_password(p: &str, min: usize) -> Result<(), AppError> {
    let n = p.chars().count();
    if n < min || n > MAX_PASSWORD {
        return Err(AppError::bad_field("password"));
    }
    Ok(())
}

fn bad_credentials() -> AppError {
    AppError::new(StatusCode::UNAUTHORIZED, "bad_credentials")
}

fn account_key(id: i64) -> Key {
    Key::try_from(id).unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Accounts and sessions
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct Account {
    pub id: i64,
    pub username: String,
    pub pass_hash: String,
    pub is_admin: bool,
    pub is_perm: bool,
    pub expires_at: Option<i64>,
}

pub fn find_account(c: &Connection, username: &str) -> Result<Option<Account>, AppError> {
    let mut stmt = c.prepare_cached(
        "SELECT id, username, pass_hash, is_admin, is_perm, expires_at FROM accounts WHERE username = ?1",
    )?;
    opt(stmt.query_row([username], |r| {
        Ok(Account {
            id: r.get(0)?,
            username: r.get(1)?,
            pass_hash: r.get(2)?,
            is_admin: r.get::<_, i64>(3)? != 0,
            is_perm: r.get::<_, i64>(4)? != 0,
            expires_at: r.get(5)?,
        })
    }))
}

#[derive(Clone, Debug)]
pub struct Session {
    pub account_id: i64,
    pub username: String,
    pub is_admin: bool,
    pub is_perm: bool,
    pub expires_at: Option<i64>,
    pub token: String,
    pub token_hash: [u8; 32],
}

pub fn account_json(
    username: &str,
    is_admin: bool,
    is_perm: bool,
    expires_at: Option<i64>,
) -> Value {
    json!({
        "username": username,
        "is_admin": is_admin,
        "is_perm": is_perm,
        "expires_at": expires_at,
    })
}

/// Whether the session cookie is (to be) marked Secure for this request.
pub fn secure_cookie(cfg: &Config, headers: &HeaderMap) -> bool {
    match cfg.secure_cookie {
        SecureCookie::Auto => is_https(cfg, headers),
        SecureCookie::Always => true,
        SecureCookie::Never => false,
    }
}

fn cookie_name(secure: bool) -> &'static str {
    if secure {
        SECURE_SESSION_COOKIE
    } else {
        SESSION_COOKIE
    }
}

/// The session token from the session cookie, if it is well formed. Only
/// the name that fits the request is read. When the cookie arrives more
/// than once with different values (someone else planted one beside ours)
/// none of them is trusted.
pub fn cookie_token(headers: &HeaderMap, secure: bool) -> Option<String> {
    let name = cookie_name(secure);
    let mut found: Option<&str> = None;
    let pairs = headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|kv| kv.trim().split_once('='));
    for (k, v) in pairs {
        if k != name || !is_token(v) {
            continue;
        }
        match found {
            None => found = Some(v),
            Some(f) if f == v => {}
            Some(_) => return None,
        }
    }
    found.map(str::to_owned)
}

/// Looks the session up by the hash of its token. Expired sessions and
/// sessions of expired accounts do not count.
pub async fn session(app: &App, headers: &HeaderMap) -> Result<Option<Session>, AppError> {
    let Some(token) = cookie_token(headers, secure_cookie(&app.cfg, headers)) else {
        return Ok(None);
    };
    let hash = hash_token(&token);
    let now = now_ms();
    app.db
        .call(move |c| {
            let mut stmt = c.prepare_cached(
                "SELECT a.id, a.username, a.is_admin, a.is_perm, a.expires_at \
                 FROM sessions s JOIN accounts a ON a.id = s.account_id \
                 WHERE s.token_hash = ?1 AND s.expires_at > ?2 \
                 AND (a.expires_at IS NULL OR a.expires_at > ?2)",
            )?;
            let row = opt(stmt.query_row((&hash[..], now), |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, i64>(2)? != 0,
                    r.get::<_, i64>(3)? != 0,
                    r.get::<_, Option<i64>>(4)?,
                ))
            }))?;
            Ok(row.map(
                |(account_id, username, is_admin, is_perm, expires_at)| Session {
                    account_id,
                    username,
                    is_admin,
                    is_perm,
                    expires_at,
                    token,
                    token_hash: hash,
                },
            ))
        })
        .await
}

pub async fn require_session(app: &App, headers: &HeaderMap) -> Result<Session, AppError> {
    session(app, headers)
        .await?
        .ok_or_else(AppError::unauthorized)
}

pub async fn require_admin(app: &App, headers: &HeaderMap) -> Result<Session, AppError> {
    let s = require_session(app, headers).await?;
    if s.is_admin {
        Ok(s)
    } else {
        Err(AppError::forbidden())
    }
}

fn session_cookie(app: &App, ctx: &Ctx, value: &str, max_age: i64) -> String {
    let secure = match app.cfg.secure_cookie {
        SecureCookie::Auto => ctx.https,
        SecureCookie::Always => true,
        SecureCookie::Never => false,
    };
    format!(
        "{}={value}; Path=/; Max-Age={max_age}; HttpOnly; SameSite=Lax{}",
        cookie_name(secure),
        if secure { "; Secure" } else { "" }
    )
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct LoginReq {
    username: String,
    password: String,
    #[serde(default)]
    admin: bool,
}

pub async fn login(app: &Arc<App>, ctx: &Ctx, req: Req) -> HResult {
    // a client already out of attempts is turned away before its body is read
    app.limits
        .login_fail
        .check(ctx.ip)
        .map_err(AppError::rate_limited)?;
    let body: LoginReq = read_json(req, JSON_LIMIT, false).await?;
    let username = valid_username(&body.username)?;
    valid_password(&body.password, 1)?;
    // Take a failure token up front and give it back on success, so a burst
    // of parallel guesses cannot all pass the check.
    app.limits
        .login_fail
        .take(ctx.ip)
        .map_err(AppError::rate_limited)?;
    let result = login_inner(app, ctx, body, username).await;
    // only a wrong password (or unknown admin name) keeps the token
    if !matches!(&result, Err(e) if e.code == "bad_credentials") {
        app.limits.login_fail.refund(ctx.ip);
    }
    result
}

async fn login_inner(app: &Arc<App>, ctx: &Ctx, body: LoginReq, username: String) -> HResult {
    let now = now_ms();
    let lookup = username.clone();
    let existing = app.db.call(move |c| find_account(c, &lookup)).await?;
    let account = match existing {
        Some(a) => a,
        None if body.admin => {
            // same cost as a real check, so timing does not reveal names
            let _ = verify_password(app, body.password, app.dummy_hash.clone()).await?;
            return Err(bad_credentials());
        }
        None => {
            if !app.settings().registration {
                return Err(AppError::new(StatusCode::FORBIDDEN, "registration_closed"));
            }
            app.limits
                .register
                .take(ctx.ip)
                .map_err(AppError::rate_limited)?;
            let hash = hash_password(app, body.password.clone()).await?;
            let name = username.clone();
            let (account, created) = app
                .db
                .call(move |c| {
                    let n = c
                        .prepare_cached(
                            "INSERT INTO accounts(username, pass_hash, is_admin, is_perm, created_at, expires_at) \
                             VALUES (?1, ?2, 0, 0, ?3, ?4) ON CONFLICT(username) DO NOTHING",
                        )?
                        .execute((&name, &hash, now, now + WEEK_MS))?;
                    let acc = find_account(c, &name)?
                        .ok_or_else(|| AppError::internal("register", "account vanished"))?;
                    Ok((acc, n == 1))
                })
                .await?;
            if created {
                return finish_login(app, ctx, account, now).await;
            }
            // lost a race with a concurrent registration: check the password
            account
        }
    };
    // Guesses at one account from many addresses share a budget too. When
    // it runs dry, a client that has not failed recently may still try
    // (see `Guarded`), so the budget cannot be used to lock the owner out.
    let ticket = app
        .limits
        .login_account
        .take(account_key(account.id), ctx.ip)
        .map_err(AppError::rate_limited)?;
    if !verify_password(app, body.password, account.pass_hash.clone()).await? {
        return Err(bad_credentials());
    }
    app.limits.login_account.refund(ticket);
    if body.admin && !account.is_admin {
        return Err(AppError::forbidden());
    }
    finish_login(app, ctx, account, now).await
}

async fn finish_login(app: &App, ctx: &Ctx, acc: Account, now: i64) -> HResult {
    let token = new_token()?;
    let hash = token.hash;
    let id = acc.id;
    let renew = !acc.is_admin && !acc.is_perm && acc.expires_at.is_none_or(|e| e <= now);
    let expires_at = app
        .db
        .call(move |c| {
            let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
            if renew {
                tx.execute(
                    "UPDATE accounts SET expires_at = ?1 WHERE id = ?2",
                    (now + WEEK_MS, id),
                )?;
            }
            tx.execute(
                "INSERT INTO sessions(token_hash, account_id, expires_at) VALUES (?1, ?2, ?3)",
                (&hash[..], id, now + WEEK_MS),
            )?;
            tx.execute(
                "DELETE FROM sessions WHERE account_id = ?1 AND token_hash NOT IN \
                 (SELECT token_hash FROM sessions WHERE account_id = ?1 ORDER BY expires_at DESC LIMIT ?2)",
                (id, MAX_SESSIONS_PER_ACCOUNT),
            )?;
            let exp: Option<i64> =
                tx.query_row("SELECT expires_at FROM accounts WHERE id = ?1", [id], |r| r.get(0))?;
            tx.commit()?;
            Ok(exp)
        })
        .await?;
    let mut resp = json_response(
        StatusCode::OK,
        &account_json(&acc.username, acc.is_admin, acc.is_perm, expires_at),
    );
    put_header(
        &mut resp,
        header::SET_COOKIE,
        &session_cookie(app, ctx, &token.text, SESSION_MAX_AGE_SECS),
    )?;
    Ok(resp)
}

pub async fn logout(app: &App, ctx: &Ctx, req: &Req) -> HResult {
    if let Some(token) = cookie_token(req.headers(), secure_cookie(&app.cfg, req.headers())) {
        let hash = hash_token(&token);
        app.db
            .call(move |c| {
                c.execute("DELETE FROM sessions WHERE token_hash = ?1", [&hash[..]])?;
                Ok(())
            })
            .await?;
    }
    let mut resp = json_response(StatusCode::OK, &json!({"ok": true}));
    put_header(
        &mut resp,
        header::SET_COOKIE,
        &session_cookie(app, ctx, "", 0),
    )?;
    Ok(resp)
}

pub async fn me(app: &App, req: &Req) -> HResult {
    let s = require_session(app, req.headers()).await?;
    Ok(json_response(
        StatusCode::OK,
        &account_json(&s.username, s.is_admin, s.is_perm, s.expires_at),
    ))
}

pub async fn renew(app: &App, ctx: &Ctx, req: &Req) -> HResult {
    let s = require_session(app, req.headers()).await?;
    if s.is_admin || s.is_perm {
        return Err(AppError::new(StatusCode::BAD_REQUEST, "never_expires"));
    }
    let now = now_ms();
    let until = now + WEEK_MS;
    let (id, hash) = (s.account_id, s.token_hash);
    app.db
        .call(move |c| {
            let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
            tx.execute(
                "UPDATE accounts SET expires_at = ?1 WHERE id = ?2",
                (until, id),
            )?;
            tx.execute(
                "UPDATE sessions SET expires_at = ?1 WHERE token_hash = ?2",
                (until, &hash[..]),
            )?;
            tx.commit()?;
            Ok(())
        })
        .await?;
    let mut resp = json_response(StatusCode::OK, &json!({"expires_at": until}));
    put_header(
        &mut resp,
        header::SET_COOKIE,
        &session_cookie(app, ctx, &s.token, SESSION_MAX_AGE_SECS),
    )?;
    Ok(resp)
}

// ---------------------------------------------------------------------------
// Admin bootstrap (startup and CLI)
// ---------------------------------------------------------------------------

pub enum AdminInit {
    Existing,
    FromEnv,
    Generated(String),
}

fn check_admin_password(p: &str) -> Result<(), String> {
    let n = p.chars().count();
    if !(MIN_ADMIN_PASSWORD..=MAX_PASSWORD).contains(&n) {
        return Err(format!(
            "admin password must be {MIN_ADMIN_PASSWORD} to {MAX_PASSWORD} characters"
        ));
    }
    Ok(())
}

fn free_admin_name(c: &Connection) -> Result<String, AppError> {
    for i in 1..1000 {
        let name = if i == 1 {
            "admin".to_owned()
        } else {
            format!("admin{i}")
        };
        if find_account(c, &name)?.is_none() {
            return Ok(name);
        }
    }
    Err(AppError::internal("admin", "no free admin username"))
}

/// Creates the admin account on first start.
pub fn ensure_admin(db: &Db, env_password: Option<&str>) -> Result<AdminInit, String> {
    let err = |e: AppError| format!("admin account: {}", e.code);
    let exists = db
        .blocking(|c| {
            let n: i64 = c.query_row(
                "SELECT COUNT(*) FROM accounts WHERE is_admin = 1",
                [],
                |r| r.get(0),
            )?;
            Ok(n > 0)
        })
        .map_err(err)?;
    if exists {
        return Ok(AdminInit::Existing);
    }
    let (password, generated) = match env_password {
        Some(p) => {
            check_admin_password(p).map_err(|e| format!("OPENDROP_ADMIN_PASSWORD: {e}"))?;
            (p.to_owned(), false)
        }
        None => (random_password(16).map_err(err)?, true),
    };
    let hash = hash_blocking(&password).map_err(err)?;
    let now = now_ms();
    db.blocking(|c| {
        let name = free_admin_name(c)?;
        c.execute(
            "INSERT INTO accounts(username, pass_hash, is_admin, is_perm, created_at, expires_at) \
             VALUES (?1, ?2, 1, 0, ?3, NULL)",
            (&name, &hash, now),
        )?;
        Ok(())
    })
    .map_err(err)?;
    Ok(if generated {
        AdminInit::Generated(password)
    } else {
        AdminInit::FromEnv
    })
}

/// Sets a new admin password (random when `None`) and signs out every admin
/// session. Returns (username, password).
pub fn reset_admin(db: &Db, password: Option<String>) -> Result<(String, String), String> {
    let err = |e: AppError| format!("admin reset: {}", e.code);
    let password = match password {
        Some(p) => {
            check_admin_password(&p)?;
            p
        }
        None => random_password(16).map_err(err)?,
    };
    let hash = hash_blocking(&password).map_err(err)?;
    let now = now_ms();
    let username = db
        .blocking(|c| {
            let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let existing: Option<(i64, String)> = opt(tx.query_row(
                "SELECT id, username FROM accounts WHERE is_admin = 1 ORDER BY id LIMIT 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            ))?;
            let name = match existing {
                Some((id, name)) => {
                    tx.execute(
                        "UPDATE accounts SET pass_hash = ?1, expires_at = NULL WHERE id = ?2",
                        (&hash, id),
                    )?;
                    name
                }
                None => {
                    let name = free_admin_name(&tx)?;
                    tx.execute(
                        "INSERT INTO accounts(username, pass_hash, is_admin, is_perm, created_at, expires_at) \
                         VALUES (?1, ?2, 1, 0, ?3, NULL)",
                        (&name, &hash, now),
                    )?;
                    name
                }
            };
            tx.execute(
                "DELETE FROM sessions WHERE account_id IN (SELECT id FROM accounts WHERE is_admin = 1)",
                [],
            )?;
            tx.commit()?;
            Ok(name)
        })
        .map_err(err)?;
    Ok((username, password))
}

#[cfg(test)]
mod tests {
    use super::*;
    use http::HeaderValue;

    #[test]
    fn usernames() {
        assert_eq!(valid_username("  bob ").ok().as_deref(), Some("bob"));
        assert_eq!(valid_username("张三").ok().as_deref(), Some("张三"));
        assert!(valid_username("").is_err());
        assert!(valid_username("   ").is_err());
        assert!(valid_username("a b").is_err());
        assert!(valid_username("a\u{202E}b").is_err());
        assert!(valid_username("a\tb").is_err());
        assert!(valid_username(&"x".repeat(33)).is_err());
        assert!(valid_username(&"界".repeat(32)).is_ok());
        assert!(valid_password("", 1).is_err());
        assert!(valid_password(&"p".repeat(129), 1).is_err());
        assert!(valid_password(&"密".repeat(128), 1).is_ok());
    }

    #[test]
    fn hashing() {
        let h = hash_blocking("correct horse").expect("hash");
        assert!(h.starts_with("$argon2id$v=19$m=19456,t=2,p=1$"), "{h}");
        assert!(verify_blocking("correct horse", &h));
        assert!(!verify_blocking("wrong", &h));
        assert!(!verify_blocking("x", "not a hash"));
    }

    #[test]
    fn cookie_parsing() {
        let tok = new_token().expect("token").text;
        let other = new_token().expect("token").text;
        let cookie = |v: String| {
            let mut h = HeaderMap::new();
            h.insert(header::COOKIE, HeaderValue::from_str(&v).expect("value"));
            h
        };
        let h = cookie(format!("a=1; {SESSION_COOKIE}={tok}; b=2"));
        assert_eq!(cookie_token(&h, false).as_deref(), Some(tok.as_str()));
        // over https only the __Host- cookie counts
        assert_eq!(cookie_token(&h, true), None);
        let h = cookie(format!(
            "{SECURE_SESSION_COOKIE}={tok}; {SESSION_COOKIE}={other}"
        ));
        assert_eq!(cookie_token(&h, true).as_deref(), Some(tok.as_str()));
        assert_eq!(cookie_token(&h, false).as_deref(), Some(other.as_str()));
        // a second, different value (a tossed cookie) makes it ambiguous
        let h = cookie(format!("{SESSION_COOKIE}={other}; {SESSION_COOKIE}={tok}"));
        assert_eq!(cookie_token(&h, false), None);
        let h = cookie(format!("{SESSION_COOKIE}={tok}; {SESSION_COOKIE}={tok}"));
        assert_eq!(cookie_token(&h, false).as_deref(), Some(tok.as_str()));
        let h = cookie("od_sid=short".to_owned());
        assert_eq!(cookie_token(&h, false), None);
    }
}
