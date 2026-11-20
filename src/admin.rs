//! Admin API, plus the public /api/info numbers.

use std::sync::Arc;

use http::StatusCode;
use rusqlite::{Connection, TransactionBehavior};
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::App;
use crate::auth::{
    MIN_ADMIN_PASSWORD, find_account, hash_password, require_admin, valid_password, valid_username,
    verify_password,
};
use crate::db::{self, opt};
use crate::disk::{self, DiskInfo};
use crate::http::{Ctx, HResult, JSON_LIMIT, Req, ok_json, read_json};
use crate::parcels::{PasswordChange, STATE_READY, kind_str};
use crate::sweep::remove_parcel_dirs;
use crate::util::{AppError, WEEK_MS, after_hours, now_ms, query_param};

/// Upper bounds for admin edits (owners are held to the settings).
const ADMIN_MAX_EXPIRY_HOURS: i64 = 876_000;
const ADMIN_MAX_DOWNLOADS: i64 = 999_999;
/// Rows per page of the admin parcel and account lists (`?limit=` may ask
/// for up to `PAGE_MAX`).
pub const PAGE_DEFAULT: i64 = 500;
pub const PAGE_MAX: i64 = 1000;

async fn disk_now(app: &Arc<App>) -> Result<DiskInfo, AppError> {
    let dir = app.cfg.data_dir.clone();
    tokio::task::spawn_blocking(move || disk::disk_info(&dir))
        .await
        .map_err(|e| AppError::internal("statvfs task", e))?
        .map_err(|e| AppError::internal("statvfs", e))
}

/// `limit` and the cursor (`before` or `after`) of a list request.
fn page(req: &Req, cursor: &str) -> Result<(i64, Option<i64>), AppError> {
    let q = req.uri().query().unwrap_or("");
    let num = |key: &str| -> Result<Option<i64>, AppError> {
        match query_param(q, key) {
            None => Ok(None),
            Some(v) => v
                .parse::<i64>()
                .ok()
                .filter(|n| *n > 0)
                .map(Some)
                .ok_or_else(|| AppError::bad_field(key)),
        }
    };
    let limit = num("limit")?.unwrap_or(PAGE_DEFAULT).min(PAGE_MAX);
    Ok((limit, num(cursor)?))
}

/// GET /api/info
pub async fn public_info(app: &Arc<App>) -> HResult {
    let s = app.settings();
    let d = disk_now(app).await?;
    let used = app.db.call(|c| db::usage(c)).await?.used;
    ok_json(json!({
        "max_upload": s.max_upload_bytes,
        "max_expiry_hours": s.max_expiry_hours,
        "anon_upload": s.anon_upload,
        "registration": s.registration,
        "site_name": s.site_name,
        "used": used,
        "quota": disk::quota(&s, d.total),
    }))
}

pub async fn overview(app: &Arc<App>, req: &Req) -> HResult {
    require_admin(app, req.headers()).await?;
    let s = app.settings();
    let d = disk_now(app).await?;
    let (u, accounts) = app
        .db
        .call(|c| {
            let u = db::usage(c)?;
            let accounts: i64 = c.query_row(
                "SELECT COUNT(*) FROM accounts WHERE is_admin = 0",
                [],
                |r| r.get(0),
            )?;
            Ok((u, accounts))
        })
        .await?;
    ok_json(json!({
        "disk": {"total": d.total, "avail": d.avail},
        "used": u.used,
        "quota": disk::quota(&s, d.total),
        "reserve": disk::reserve(d.total),
        "parcels": u.parcels,
        "accounts": accounts,
        "settings": s.to_json(),
    }))
}

// ---------------------------------------------------------------------------
// Parcels
// ---------------------------------------------------------------------------

const PARCEL_COLUMNS: &str = "p.id, p.code, a.username, p.kind, \
     (SELECT COUNT(*) FROM files f WHERE f.parcel_id = p.id), p.total_bytes, p.created_at, \
     p.expires_at, p.downloads_left, p.pass_hash IS NOT NULL, p.state";

const PARCEL_FROM: &str = "FROM parcels p LEFT JOIN accounts a ON a.id = p.account_id";

fn parcel_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Value> {
    let state: i64 = r.get(10)?;
    Ok(json!({
        "id": r.get::<_, i64>(0)?,
        "code": r.get::<_, String>(1)?,
        "owner": r.get::<_, Option<String>>(2)?,
        "kind": kind_str(r.get(3)?),
        "file_count": r.get::<_, i64>(4)?,
        "total_bytes": r.get::<_, i64>(5)?,
        "created_at": r.get::<_, i64>(6)?,
        "expires_at": r.get::<_, Option<i64>>(7)?,
        "downloads_left": r.get::<_, Option<i64>>(8)?,
        "has_password": r.get::<_, bool>(9)?,
        "state": if state == STATE_READY { "ready" } else { "uploading" },
    }))
}

fn admin_parcel(c: &Connection, id: i64) -> Result<Value, AppError> {
    opt(c.query_row(
        &format!("SELECT {PARCEL_COLUMNS} {PARCEL_FROM} WHERE p.id = ?1"),
        [id],
        parcel_row,
    ))?
    .ok_or_else(AppError::not_found)
}

/// GET /api/admin/parcels[?limit=N][&before=ID]: newest first, one page.
/// `next` is the `before` value of the following page, or null.
pub async fn parcels(app: &Arc<App>, req: &Req) -> HResult {
    require_admin(app, req.headers()).await?;
    let (limit, before) = page(req, "before")?;
    let before = before.unwrap_or(i64::MAX);
    let (rows, next) = app
        .db
        .call(move |c| {
            let mut stmt = c.prepare_cached(&format!(
                "SELECT {PARCEL_COLUMNS} {PARCEL_FROM} WHERE p.id < ?1 ORDER BY p.id DESC LIMIT ?2"
            ))?;
            let rows = stmt.query_map((before, limit + 1), |r| {
                Ok((r.get::<_, i64>(0)?, parcel_row(r)?))
            })?;
            let mut out = Vec::new();
            let mut last = None;
            let mut more = false;
            for r in rows {
                let (id, v) = r?;
                if i64::try_from(out.len()).unwrap_or(i64::MAX) >= limit {
                    more = true;
                    break;
                }
                last = Some(id);
                out.push(v);
            }
            Ok((out, if more { last } else { None }))
        })
        .await?;
    ok_json(json!({"parcels": rows, "next": next}))
}

/// `Some(None)` for an explicit null, `Some(Some(n))` for a number in range.
fn nullable_int(
    body: &Map<String, Value>,
    key: &str,
    max: i64,
) -> Result<Option<Option<i64>>, AppError> {
    match body.get(key) {
        None => Ok(None),
        Some(Value::Null) => Ok(Some(None)),
        Some(v) => v
            .as_i64()
            .filter(|n| (1..=max).contains(n))
            .map(|n| Some(Some(n)))
            .ok_or_else(|| AppError::bad_field(key)),
    }
}

pub async fn parcel_patch(app: &Arc<App>, req: Req, id: i64) -> HResult {
    require_admin(app, req.headers()).await?;
    let body: Map<String, Value> = read_json(req, JSON_LIMIT, false).await?;
    let expiry = nullable_int(&body, "expiry_hours", ADMIN_MAX_EXPIRY_HOURS)?;
    let downloads = nullable_int(&body, "downloads", ADMIN_MAX_DOWNLOADS)?;
    let password = match body.get("password") {
        None | Some(Value::Null) => PasswordChange::Keep,
        Some(Value::String(p)) if p.is_empty() => PasswordChange::Clear,
        Some(Value::String(p)) => {
            if p.chars().count() > crate::parcels::MAX_PASSWORD_CHARS {
                return Err(AppError::bad_field("password"));
            }
            PasswordChange::Set(hash_password(app, p.clone()).await?)
        }
        Some(_) => return Err(AppError::bad_field("password")),
    };
    let now = now_ms();
    let row = app
        .db
        .call(move |c| {
            let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let state: Option<i64> = opt(tx.query_row(
                "SELECT state FROM parcels WHERE id = ?1",
                [id],
                |r| r.get(0),
            ))?;
            match state {
                None => return Err(AppError::not_found()),
                Some(s) if s != STATE_READY => {
                    return Err(AppError::new(StatusCode::CONFLICT, "uploading"));
                }
                Some(_) => {}
            }
            match expiry {
                None => {}
                Some(None) => {
                    tx.execute("UPDATE parcels SET expires_at = NULL WHERE id = ?1", [id])?;
                }
                Some(Some(h)) => {
                    tx.execute(
                        "UPDATE parcels SET expires_at = ?1, expiry_hours = ?2 WHERE id = ?3",
                        (after_hours(now, h), h, id),
                    )?;
                }
            }
            if let Some(d) = downloads {
                tx.execute(
                    "UPDATE parcels SET downloads_left = ?1 WHERE id = ?2",
                    (d, id),
                )?;
            }
            password.apply(&tx, id)?;
            let row = admin_parcel(&tx, id)?;
            tx.commit()?;
            Ok(row)
        })
        .await?;
    ok_json(row)
}

pub async fn parcel_delete(app: &Arc<App>, req: &Req, id: i64) -> HResult {
    require_admin(app, req.headers()).await?;
    let n = app
        .db
        .call(move |c| Ok(c.execute("DELETE FROM parcels WHERE id = ?1", [id])?))
        .await?;
    if n == 0 {
        return Err(AppError::not_found());
    }
    app.inflight.cancel_parcel(id);
    remove_parcel_dirs(app, vec![id]).await;
    ok_json(json!({"ok": true}))
}

// ---------------------------------------------------------------------------
// Accounts
// ---------------------------------------------------------------------------

const ACCOUNT_SELECT: &str = "SELECT a.id, a.username, a.is_perm, a.created_at, a.expires_at, \
     COUNT(p.id), COALESCE(SUM(p.total_bytes), 0) \
     FROM accounts a LEFT JOIN parcels p ON p.account_id = a.id WHERE a.is_admin = 0";

fn account_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Value> {
    Ok(json!({
        "id": r.get::<_, i64>(0)?,
        "username": r.get::<_, String>(1)?,
        "is_perm": r.get::<_, bool>(2)?,
        "created_at": r.get::<_, i64>(3)?,
        "expires_at": r.get::<_, Option<i64>>(4)?,
        "parcels": r.get::<_, i64>(5)?,
        "bytes": r.get::<_, i64>(6)?,
    }))
}

/// GET /api/admin/accounts[?limit=N][&after=ID]: oldest first, one page.
/// `next` is the `after` value of the following page, or null.
pub async fn accounts(app: &Arc<App>, req: &Req) -> HResult {
    require_admin(app, req.headers()).await?;
    let (limit, after) = page(req, "after")?;
    let after = after.unwrap_or(0);
    let (rows, next) = app
        .db
        .call(move |c| {
            let mut stmt = c.prepare_cached(&format!(
                "{ACCOUNT_SELECT} AND a.id > ?1 GROUP BY a.id ORDER BY a.id LIMIT ?2"
            ))?;
            let rows = stmt.query_map((after, limit + 1), |r| {
                Ok((r.get::<_, i64>(0)?, account_row(r)?))
            })?;
            let mut out = Vec::new();
            let mut last = None;
            let mut more = false;
            for r in rows {
                let (id, v) = r?;
                if i64::try_from(out.len()).unwrap_or(i64::MAX) >= limit {
                    more = true;
                    break;
                }
                last = Some(id);
                out.push(v);
            }
            Ok((out, if more { last } else { None }))
        })
        .await?;
    ok_json(json!({"accounts": rows, "next": next}))
}

#[derive(Deserialize)]
struct AccountPatch {
    is_perm: Option<bool>,
    #[serde(default)]
    renew: bool,
}

pub async fn account_patch(app: &Arc<App>, req: Req, id: i64) -> HResult {
    require_admin(app, req.headers()).await?;
    let body: AccountPatch = read_json(req, JSON_LIMIT, false).await?;
    let now = now_ms();
    let row = app
        .db
        .call(move |c| {
            let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let cur: Option<(bool, bool)> = opt(tx.query_row(
                "SELECT is_admin, is_perm FROM accounts WHERE id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            ))?;
            let (is_admin, mut is_perm) = cur.ok_or_else(AppError::not_found)?;
            if is_admin {
                return Err(AppError::not_found());
            }
            if let Some(p) = body.is_perm {
                if p && !is_perm {
                    tx.execute(
                        "UPDATE accounts SET is_perm = 1, expires_at = NULL WHERE id = ?1",
                        [id],
                    )?;
                } else if !p && is_perm {
                    // a fresh period starts when an account becomes expiring again
                    tx.execute(
                        "UPDATE accounts SET is_perm = 0, expires_at = ?1 WHERE id = ?2",
                        (now + WEEK_MS, id),
                    )?;
                }
                is_perm = p;
            }
            if body.renew {
                if is_perm {
                    return Err(AppError::new(StatusCode::BAD_REQUEST, "never_expires"));
                }
                tx.execute(
                    "UPDATE accounts SET expires_at = ?1 WHERE id = ?2",
                    (now + WEEK_MS, id),
                )?;
            }
            let row = tx.query_row(
                &format!("{ACCOUNT_SELECT} AND a.id = ?1 GROUP BY a.id"),
                [id],
                account_row,
            )?;
            tx.commit()?;
            Ok(row)
        })
        .await?;
    ok_json(row)
}

pub async fn account_delete(app: &Arc<App>, req: &Req, id: i64) -> HResult {
    require_admin(app, req.headers()).await?;
    let ids = app
        .db
        .call(move |c| {
            let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let is_admin: Option<bool> = opt(tx.query_row(
                "SELECT is_admin FROM accounts WHERE id = ?1",
                [id],
                |r| r.get(0),
            ))?;
            match is_admin {
                None => return Err(AppError::not_found()),
                Some(true) => return Err(AppError::forbidden()),
                Some(false) => {}
            }
            let mut ids = Vec::new();
            {
                let mut stmt = tx.prepare_cached("SELECT id FROM parcels WHERE account_id = ?1")?;
                for r in stmt.query_map([id], |r| r.get::<_, i64>(0))? {
                    ids.push(r?);
                }
            }
            tx.execute("DELETE FROM parcels WHERE account_id = ?1", [id])?;
            tx.execute("DELETE FROM accounts WHERE id = ?1", [id])?;
            tx.commit()?;
            Ok(ids)
        })
        .await?;
    for &pid in &ids {
        app.inflight.cancel_parcel(pid);
    }
    remove_parcel_dirs(app, ids).await;
    ok_json(json!({"ok": true}))
}

// ---------------------------------------------------------------------------
// Settings and credentials
// ---------------------------------------------------------------------------

pub async fn settings_put(app: &Arc<App>, req: Req) -> HResult {
    require_admin(app, req.headers()).await?;
    let body: Map<String, Value> = read_json(req, JSON_LIMIT, false).await?;
    let d = disk_now(app).await?;
    // Read, apply, save and publish under the database lock, so two saves
    // at once can neither undo each other's change nor leave memory and the
    // database disagreeing.
    let app2 = Arc::clone(app);
    let out = app
        .db
        .call(move |c| {
            let next = app2.settings().apply(&body, d.total)?;
            next.save(c)?;
            let out = next.to_json();
            app2.set_settings(next);
            Ok(out)
        })
        .await?;
    ok_json(out)
}

#[derive(Deserialize)]
struct CredentialsReq {
    current: String,
    username: Option<String>,
    password: Option<String>,
}

pub async fn credentials(app: &Arc<App>, ctx: &Ctx, req: Req) -> HResult {
    let s = require_admin(app, req.headers()).await?;
    let body: CredentialsReq = read_json(req, JSON_LIMIT, false).await?;
    let new_name = match body.username.as_deref().map(str::trim) {
        None | Some("") => None,
        Some(u) => {
            let u = valid_username(u)?;
            (u != s.username).then_some(u)
        }
    };
    let new_pass = match body.password.as_deref() {
        None | Some("") => None,
        Some(p) => {
            valid_password(p, MIN_ADMIN_PASSWORD)?;
            Some(p.to_owned())
        }
    };
    if new_name.is_none() && new_pass.is_none() {
        return Err(AppError::new(StatusCode::BAD_REQUEST, "no_changes"));
    }

    app.limits
        .login_fail
        .take(ctx.ip)
        .map_err(AppError::rate_limited)?;
    let lookup = s.username.clone();
    let account = app
        .db
        .call(move |c| find_account(c, &lookup))
        .await?
        .ok_or_else(AppError::unauthorized)?;
    if !verify_password(app, body.current, account.pass_hash.clone()).await? {
        return Err(AppError::new(StatusCode::UNAUTHORIZED, "bad_credentials"));
    }
    app.limits.login_fail.refund(ctx.ip);

    let new_hash = match new_pass {
        Some(p) => Some(hash_password(app, p).await?),
        None => None,
    };
    let (id, keep) = (s.account_id, s.token_hash);
    let username = app
        .db
        .call(move |c| {
            let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
            if let Some(name) = &new_name {
                let taken: Option<i64> = opt(tx.query_row(
                    "SELECT id FROM accounts WHERE username = ?1 AND id != ?2",
                    (name, id),
                    |r| r.get(0),
                ))?;
                if taken.is_some() {
                    return Err(AppError::new(StatusCode::CONFLICT, "username_taken"));
                }
                tx.execute(
                    "UPDATE accounts SET username = ?1 WHERE id = ?2",
                    (name, id),
                )?;
            }
            if let Some(h) = &new_hash {
                tx.execute("UPDATE accounts SET pass_hash = ?1 WHERE id = ?2", (h, id))?;
            }
            // every other admin session ends; this one stays signed in
            tx.execute(
                "DELETE FROM sessions WHERE token_hash != ?1 \
                 AND account_id IN (SELECT id FROM accounts WHERE is_admin = 1)",
                [&keep[..]],
            )?;
            let name: String =
                tx.query_row("SELECT username FROM accounts WHERE id = ?1", [id], |r| {
                    r.get(0)
                })?;
            tx.commit()?;
            Ok(name)
        })
        .await?;
    ok_json(json!({"username": username}))
}
