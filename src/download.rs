//! Receiving: look a code up, claim a download, stream files or a zip with
//! Range and HEAD support.

use std::collections::HashMap;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};

use http::{HeaderName, Method, Response, StatusCode, header};
use rusqlite::{Connection, TransactionBehavior};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::App;
use crate::auth::verify_password;
use crate::body::{Body, Hold, Piece};
use crate::db::opt;
use crate::http::{
    Ctx, HResult, JSON_LIMIT, Req, error_response, header_str, ok_json, put_header, read_json,
};
use crate::parcels::{KIND_TEXT, MAX_PASSWORD_CHARS, MAX_TEXT_BYTES, STATE_READY, kind_str};
use crate::util::{
    AppError, HOUR_MS, basename, content_disposition, hash_token, is_code, is_token, new_token,
    now_ms,
};
use crate::zip;

const DOWNLOAD_CSP: &str = "sandbox; default-src 'none'";
pub const CLAIM_TTL_MS: i64 = HOUR_MS;
/// Retry-After for a client that already has its maximum of downloads open.
const BUSY_RETRY_SECS: u64 = 5;

#[derive(Deserialize, Default)]
struct PasswordReq {
    password: Option<String>,
}

struct Found {
    id: i64,
    kind: i64,
    pass_hash: Option<String>,
    total: i64,
    expires_at: Option<i64>,
    downloads_left: Option<i64>,
}

fn gone(code: &'static str) -> AppError {
    AppError::new(StatusCode::GONE, code)
}

fn parcel_key(id: i64) -> u128 {
    u128::try_from(id).unwrap_or(0)
}

fn check_code(code: &str) -> Result<(), AppError> {
    if is_code(code) {
        Ok(())
    } else {
        Err(AppError::new(StatusCode::BAD_REQUEST, "bad_code"))
    }
}

// ---------------------------------------------------------------------------
// Streams in progress
// ---------------------------------------------------------------------------

#[derive(Default)]
struct StreamCount {
    n: usize,
    doomed: bool,
}

/// Parcels with a response streaming right now. Deleting such a parcel
/// (sweeper, owner, admin) removes its rows at once, but its directory stays
/// until the last stream ends, so a download already under way always
/// completes (a zip opens each file only when it gets there).
#[derive(Default)]
pub struct Streams {
    inner: Mutex<HashMap<i64, StreamCount>>,
}

/// Keeps a parcel's directory in place while held.
pub struct StreamHold {
    streams: Arc<Streams>,
    id: i64,
    dir: PathBuf,
}

impl Streams {
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<i64, StreamCount>> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn begin(self: &Arc<Self>, id: i64, dir: PathBuf) -> StreamHold {
        let mut m = self.lock();
        let c = m.entry(id).or_default();
        c.n = c.n.saturating_add(1);
        drop(m);
        StreamHold {
            streams: Arc::clone(self),
            id,
            dir,
        }
    }

    /// True when `id` is streaming; its directory is then removed by the
    /// last stream instead of now.
    pub fn defer_removal(&self, id: i64) -> bool {
        match self.lock().get_mut(&id) {
            Some(c) => {
                c.doomed = true;
                true
            }
            None => false,
        }
    }

    pub fn is_active(&self, id: i64) -> bool {
        self.lock().contains_key(&id)
    }
}

impl Drop for StreamHold {
    fn drop(&mut self) {
        let remove = {
            let mut m = self.streams.lock();
            match m.get_mut(&self.id) {
                Some(c) => {
                    c.n = c.n.saturating_sub(1);
                    let last = c.n == 0;
                    let doomed = c.doomed;
                    if last {
                        m.remove(&self.id);
                    }
                    last && doomed
                }
                None => false,
            }
        };
        if remove {
            let dir = std::mem::take(&mut self.dir);
            let work = move || match std::fs::remove_dir_all(&dir) {
                Err(e) if e.kind() != io::ErrorKind::NotFound => {
                    crate::log!("warning: removing parcel files: {}: {e}", dir.display());
                }
                _ => {}
            };
            match tokio::runtime::Handle::try_current() {
                Ok(rt) => drop(rt.spawn_blocking(work)),
                Err(_) => work(),
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Lookup and claim
// ---------------------------------------------------------------------------

/// Finds a ready, live parcel by code and checks its password, applying the
/// miss and password rate limits. The caller charges the lookup bucket
/// before reading the request body.
async fn open_parcel(
    app: &Arc<App>,
    ctx: &Ctx,
    code: &str,
    password: Option<String>,
) -> Result<Found, AppError> {
    let limits = &app.limits;
    // A miss costs from the client's buckets (and those of its wider IPv6
    // networks) and from the server wide budget. The tokens are taken up
    // front and given back on a hit, so parallel probes cannot outrun the
    // brake.
    limits.miss.take(ctx.ip).map_err(AppError::rate_limited)?;
    let ticket = match limits.miss_global.take(0, ctx.ip) {
        Ok(t) => t,
        Err(secs) => {
            limits.miss.refund(ctx.ip);
            return Err(AppError::rate_limited(secs));
        }
    };
    let code_owned = code.to_owned();
    let found = app
        .db
        .call(move |c| {
            opt(c
                .prepare_cached(
                    "SELECT id, kind, pass_hash, total_bytes, expires_at, downloads_left \
                     FROM parcels WHERE code = ?1 AND state = 1",
                )?
                .query_row([&code_owned], |r| {
                    Ok(Found {
                        id: r.get(0)?,
                        kind: r.get(1)?,
                        pass_hash: r.get(2)?,
                        total: r.get(3)?,
                        expires_at: r.get(4)?,
                        downloads_left: r.get(5)?,
                    })
                }))
        })
        .await;
    let found = match found {
        Ok(Some(p)) => {
            limits.miss.refund(ctx.ip);
            limits.miss_global.refund(ticket);
            p
        }
        Ok(None) => return Err(AppError::not_found()),
        Err(e) => {
            limits.miss.refund(ctx.ip);
            limits.miss_global.refund(ticket);
            return Err(e);
        }
    };
    let now = now_ms();
    if found.expires_at.is_some_and(|e| e <= now) {
        return Err(gone("expired"));
    }
    if found.downloads_left == Some(0) {
        return Err(gone("exhausted"));
    }
    if let Some(hash) = &found.pass_hash {
        let pw = password
            .filter(|p| !p.is_empty())
            .ok_or_else(|| AppError::new(StatusCode::UNAUTHORIZED, "password_required"))?;
        let key = parcel_key(found.id);
        limits.pw_ip.take(ctx.ip).map_err(AppError::rate_limited)?;
        if let Err(secs) = limits.pw_parcel.take(key) {
            limits.pw_ip.refund(ctx.ip);
            return Err(AppError::rate_limited(secs));
        }
        let ok = pw.chars().count() <= MAX_PASSWORD_CHARS
            && verify_password(app, pw, hash.clone()).await?;
        if !ok {
            return Err(AppError::new(StatusCode::UNAUTHORIZED, "bad_password"));
        }
        limits.pw_ip.refund(ctx.ip);
        limits.pw_parcel.refund(key);
    }
    Ok(found)
}

pub async fn info(app: &Arc<App>, ctx: &Ctx, req: Req, code: &str) -> HResult {
    check_code(code)?;
    app.limits
        .lookup
        .take(ctx.ip)
        .map_err(AppError::rate_limited)?;
    let body: PasswordReq = read_json(req, JSON_LIMIT, true).await?;
    let p = open_parcel(app, ctx, code, body.password).await?;
    let id = p.id;
    let files = app
        .db
        .call(move |c| {
            let mut stmt =
                c.prepare_cached("SELECT idx, name, size FROM files WHERE parcel_id = ?1 ORDER BY idx")?;
            let rows = stmt.query_map([id], |r| {
                Ok(json!({"i": r.get::<_, i64>(0)?, "name": r.get::<_, String>(1)?, "size": r.get::<_, i64>(2)?}))
            })?;
            let mut out = Vec::new();
            for r in rows {
                out.push(r?);
            }
            Ok(out)
        })
        .await?;
    ok_json(json!({
        "kind": kind_str(p.kind),
        "files": files,
        "total_bytes": p.total,
        "expires_at": p.expires_at,
        "downloads_left": p.downloads_left,
        "has_password": p.pass_hash.is_some(),
    }))
}

fn read_text(path: PathBuf) -> io::Result<String> {
    let f = File::open(path)?;
    let mut buf = Vec::new();
    f.take(MAX_TEXT_BYTES as u64 + 1).read_to_end(&mut buf)?;
    if buf.len() > MAX_TEXT_BYTES {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "text too long"));
    }
    String::from_utf8(buf).map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "text not UTF-8"))
}

pub async fn claim(app: &Arc<App>, ctx: &Ctx, req: Req, code: &str) -> HResult {
    check_code(code)?;
    app.limits
        .lookup
        .take(ctx.ip)
        .map_err(AppError::rate_limited)?;
    let body: PasswordReq = read_json(req, JSON_LIMIT, true).await?;
    let p = open_parcel(app, ctx, code, body.password).await?;
    let id = p.id;
    // read the text first so a broken file never costs a download
    let text = if p.kind == KIND_TEXT {
        let path = app.parcel_dir(id).join("0");
        match tokio::task::spawn_blocking(move || read_text(path)).await {
            Ok(Ok(t)) => Some(t),
            Ok(Err(e)) if e.kind() == io::ErrorKind::NotFound => return Err(gone("gone")),
            Ok(Err(e)) => return Err(AppError::internal("read text", e)),
            Err(e) => return Err(AppError::internal("read text", e)),
        }
    } else {
        None
    };
    let token = new_token()?;
    let hash = token.hash;
    let now = now_ms();
    let expires_at = now + CLAIM_TTL_MS;
    let left = app
        .db
        .call(move |c| {
            let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let left: Option<Option<i64>> = opt(tx.query_row(
                "UPDATE parcels SET downloads_left = CASE WHEN downloads_left IS NULL THEN NULL \
                 ELSE downloads_left - 1 END, touched_at = ?2 \
                 WHERE id = ?1 AND state = 1 AND (downloads_left IS NULL OR downloads_left > 0) \
                 AND (expires_at IS NULL OR expires_at > ?2) RETURNING downloads_left",
                (id, now),
                |r| r.get(0),
            ))?;
            let Some(left) = left else {
                // lost a race: tell which way
                let row: Option<Option<i64>> = opt(tx.query_row(
                    "SELECT expires_at FROM parcels WHERE id = ?1",
                    [id],
                    |r| r.get(0),
                ))?;
                return Err(match row {
                    None => AppError::not_found(),
                    Some(Some(e)) if e <= now => gone("expired"),
                    Some(_) => gone("exhausted"),
                });
            };
            tx.execute(
                "INSERT INTO claims(token_hash, parcel_id, expires_at) VALUES (?1, ?2, ?3)",
                (&hash[..], id, expires_at),
            )?;
            tx.commit()?;
            Ok(left)
        })
        .await?;
    let mut v = json!({"token": token.text, "expires_at": expires_at, "downloads_left": left});
    if let (Some(t), Value::Object(m)) = (text, &mut v) {
        m.insert("text".to_owned(), Value::String(t));
    }
    ok_json(v)
}

// ---------------------------------------------------------------------------
// Range
// ---------------------------------------------------------------------------

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum RangeReq {
    /// No (usable) Range header: send everything.
    Full,
    /// Inclusive byte range.
    Part(u64, u64),
    Unsatisfiable,
}

/// Parses a single `bytes=` range against a representation of `len` bytes.
/// Several ranges or a malformed header are ignored (full response).
pub fn parse_range(header: &str, len: u64) -> RangeReq {
    let Some(spec) = header.trim().strip_prefix("bytes=") else {
        return RangeReq::Full;
    };
    if spec.contains(',') {
        return RangeReq::Full;
    }
    let Some((a, b)) = spec.trim().split_once('-') else {
        return RangeReq::Full;
    };
    let (a, b) = (a.trim(), b.trim());
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|c| c.is_ascii_digit());
    if a.is_empty() {
        // suffix: last n bytes
        if !digits(b) {
            return RangeReq::Full;
        }
        let Ok(n) = b.parse::<u64>() else {
            return RangeReq::Full;
        };
        if n == 0 || len == 0 {
            return RangeReq::Unsatisfiable;
        }
        return RangeReq::Part(len.saturating_sub(n), len - 1);
    }
    if !digits(a) || !(b.is_empty() || digits(b)) {
        return RangeReq::Full;
    }
    let Ok(start) = a.parse::<u64>() else {
        return RangeReq::Unsatisfiable; // more digits than u64: beyond any file
    };
    let end = if b.is_empty() {
        u64::MAX
    } else {
        b.parse::<u64>().unwrap_or(u64::MAX)
    };
    if end < start {
        return RangeReq::Full;
    }
    if start >= len {
        return RangeReq::Unsatisfiable;
    }
    RangeReq::Part(start, end.min(len - 1))
}

// ---------------------------------------------------------------------------
// Download
// ---------------------------------------------------------------------------

struct FileRow {
    idx: i64,
    name: String,
    size: u64,
    crc32: u32,
}

struct ClaimRow {
    parcel: i64,
    code: String,
    created_at: i64,
}

/// The parcel a live claim token is for (403 bad_token, 410 gone).
fn claim_row(c: &Connection, hash: &[u8; 32], now: i64) -> Result<ClaimRow, AppError> {
    let row: Option<(i64, String, i64, Option<i64>, i64, i64)> = opt(c
        .prepare_cached(
            "SELECT p.id, p.code, p.state, p.expires_at, p.created_at, c.expires_at \
             FROM claims c JOIN parcels p ON p.id = c.parcel_id WHERE c.token_hash = ?1",
        )?
        .query_row([&hash[..]], |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
            ))
        }))?;
    let Some((parcel, code, state, p_exp, created_at, c_exp)) = row else {
        return Err(AppError::bad_token());
    };
    if c_exp <= now {
        return Err(AppError::bad_token());
    }
    if state != STATE_READY || p_exp.is_some_and(|e| e <= now) {
        return Err(gone("gone"));
    }
    Ok(ClaimRow {
        parcel,
        code,
        created_at,
    })
}

/// File rows of a parcel: one index, or all of them in order.
fn file_rows(c: &Connection, parcel: i64, target: Option<u32>) -> Result<Vec<FileRow>, AppError> {
    let map = |r: &rusqlite::Row<'_>| -> rusqlite::Result<FileRow> {
        Ok(FileRow {
            idx: r.get(0)?,
            name: r.get(1)?,
            size: u64::try_from(r.get::<_, i64>(2)?).unwrap_or(0),
            crc32: u32::try_from(r.get::<_, Option<i64>>(3)?.unwrap_or(0)).unwrap_or(0),
        })
    };
    let mut files = Vec::new();
    match target {
        Some(i) => {
            let mut stmt = c.prepare_cached(
                "SELECT idx, name, size, crc32 FROM files WHERE parcel_id = ?1 AND idx = ?2",
            )?;
            for r in stmt.query_map((parcel, i), map)? {
                files.push(r?);
            }
        }
        None => {
            let mut stmt = c.prepare_cached(
                "SELECT idx, name, size, crc32 FROM files WHERE parcel_id = ?1 ORDER BY idx",
            )?;
            for r in stmt.query_map([parcel], map)? {
                files.push(r?);
            }
        }
    }
    Ok(files)
}

fn open_at(path: PathBuf, start: u64, size: u64) -> Result<File, AppError> {
    let mut f = match File::open(&path) {
        Ok(f) => f,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Err(gone("gone")),
        Err(e) => return Err(AppError::internal("open download", e)),
    };
    let len = f
        .metadata()
        .map_err(|e| AppError::internal("stat download", e))?
        .len();
    if len != size {
        return Err(gone("gone"));
    }
    if start > 0 {
        f.seek(SeekFrom::Start(start))
            .map_err(|e| AppError::internal("seek download", e))?;
    }
    Ok(f)
}

/// GET or HEAD /api/dl/{token}/{idx|zip}
pub async fn download(app: &Arc<App>, ctx: &Ctx, req: &Req, token: &str, which: &str) -> HResult {
    if !is_token(token) {
        return Err(AppError::bad_token());
    }
    let target: Option<u32> = if which == "zip" {
        None
    } else {
        match which.parse::<u32>() {
            Ok(i) if i.to_string() == which => Some(i),
            _ => return Err(AppError::not_found()),
        }
    };
    let hash = hash_token(token);
    let now = now_ms();
    let first = app.db.call(move |c| claim_row(c, &hash, now)).await?;
    let parcel = first.parcel;
    let dir = app.parcel_dir(parcel);
    // From here on, deleting the parcel leaves its files in place until this
    // response is done. The claim is checked again below, so a deletion that
    // came first is still noticed.
    let hold = app.streams.begin(parcel, dir.clone());
    let head = req.method() == Method::HEAD;
    let slot = if head {
        None
    } else {
        let mine = app
            .slots
            .downloads
            .try_acquire(ctx.ip)
            .ok_or_else(|| AppError::rate_limited(BUSY_RETRY_SECS))?;
        let any = Arc::clone(&app.download_sem)
            .try_acquire_owned()
            .map_err(|_| AppError::busy(BUSY_RETRY_SECS))?;
        Some((mine, any))
    };
    let key = (parcel, first.created_at);
    let cached = if target.is_none() {
        app.zips.get(key)
    } else {
        None
    };
    let need_files = cached.is_none();
    let (claimed, files) = app
        .db
        .call(move |c| {
            let row = claim_row(c, &hash, now)?;
            if row.parcel != parcel {
                return Err(AppError::bad_token());
            }
            let files = if need_files {
                file_rows(c, parcel, target)?
            } else {
                Vec::new()
            };
            Ok((row, files))
        })
        .await?;
    let hold: Hold = Box::new((hold, slot));
    match target {
        Some(_) => {
            let f = files.into_iter().next().ok_or_else(AppError::not_found)?;
            let etag = format!("\"{}-{}-{}\"", parcel, f.idx, claimed.created_at);
            let range = requested_range(req, &etag, f.size);
            let (start, len) = match range_bounds(range, f.size) {
                Some(v) => v,
                None => return unsatisfiable(f.size),
            };
            let path = dir.join(f.idx.to_string());
            let p2 = path.clone();
            let size = f.size;
            let file = tokio::task::spawn_blocking(move || open_at(p2, start, size))
                .await
                .map_err(|e| AppError::internal("open task", e))??;
            let body = if head {
                Body::Empty
            } else {
                let piece = Piece::File {
                    file: Some(file),
                    path,
                    offset: start,
                    len,
                };
                Body::stream_from(len, Box::new(std::iter::once(piece)), Some(hold))
            };
            let meta = Meta {
                ctype: "application/octet-stream",
                filename: basename(&f.name).to_owned(),
                etag,
                total: f.size,
                start,
                len,
                partial: matches!(range, RangeReq::Part(..)),
            };
            meta.respond(body)
        }
        None => {
            let plan = match cached {
                Some(p) => p,
                None => {
                    if files.len() < 2 {
                        return Err(AppError::not_found());
                    }
                    let entries: Vec<zip::Entry> = files
                        .into_iter()
                        .map(|f| zip::Entry {
                            path: dir.join(f.idx.to_string()),
                            name: f.name,
                            size: f.size,
                            crc32: f.crc32,
                        })
                        .collect();
                    let plan = zip::Plan::new(entries, claimed.created_at)
                        .ok_or_else(|| AppError::internal("zip", "layout overflow"))?;
                    let plan = Arc::new(plan);
                    app.zips.insert(key, &plan);
                    plan
                }
            };
            let check = Arc::clone(&plan);
            let present = tokio::task::spawn_blocking(move || {
                check
                    .files()
                    .all(|(p, size)| std::fs::metadata(p).is_ok_and(|m| m.len() == size))
            })
            .await
            .map_err(|e| AppError::internal("stat task", e))?;
            if !present {
                return Err(gone("gone"));
            }
            let etag = format!("\"{}-zip-{}\"", parcel, claimed.created_at);
            let range = requested_range(req, &etag, plan.total);
            let (start, len) = match range_bounds(range, plan.total) {
                Some(v) => v,
                None => return unsatisfiable(plan.total),
            };
            let body = if head {
                Body::Empty
            } else {
                let end = start.saturating_add(len);
                Body::stream_from(len, Box::new(plan.pieces(start, end)), Some(hold))
            };
            let meta = Meta {
                ctype: "application/zip",
                filename: format!("opendrop-{}.zip", claimed.code),
                etag,
                total: plan.total,
                start,
                len,
                partial: matches!(range, RangeReq::Part(..)),
            };
            meta.respond(body)
        }
    }
}

/// The Range to honour; ignored when If-Range names another version.
fn requested_range(req: &Req, etag: &str, len: u64) -> RangeReq {
    let Some(range) = header_str(req.headers(), "range") else {
        return RangeReq::Full;
    };
    if header_str(req.headers(), "if-range").is_some_and(|v| v.trim() != etag) {
        return RangeReq::Full;
    }
    parse_range(range, len)
}

/// (start, length) to send, or `None` for 416.
fn range_bounds(r: RangeReq, total: u64) -> Option<(u64, u64)> {
    match r {
        RangeReq::Full => Some((0, total)),
        RangeReq::Part(a, b) => Some((a, b - a + 1)),
        RangeReq::Unsatisfiable => None,
    }
}

fn unsatisfiable(total: u64) -> HResult {
    let mut resp = error_response(&AppError::new(
        StatusCode::RANGE_NOT_SATISFIABLE,
        "bad_range",
    ));
    put_header(
        &mut resp,
        header::CONTENT_RANGE,
        &format!("bytes */{total}"),
    )?;
    Ok(resp)
}

struct Meta {
    ctype: &'static str,
    filename: String,
    etag: String,
    total: u64,
    start: u64,
    len: u64,
    partial: bool,
}

impl Meta {
    fn respond(self, body: Body) -> HResult {
        let mut resp = Response::new(body);
        *resp.status_mut() = if self.partial {
            StatusCode::PARTIAL_CONTENT
        } else {
            StatusCode::OK
        };
        let pairs: [(HeaderName, String); 5] = [
            (header::CONTENT_TYPE, self.ctype.to_owned()),
            (
                header::CONTENT_DISPOSITION,
                content_disposition(&self.filename),
            ),
            (header::CONTENT_LENGTH, self.len.to_string()),
            (header::ACCEPT_RANGES, "bytes".to_owned()),
            (header::ETAG, self.etag.clone()),
        ];
        for (k, v) in pairs {
            put_header(&mut resp, k, &v)?;
        }
        put_header(&mut resp, header::CONTENT_SECURITY_POLICY, DOWNLOAD_CSP)?;
        if self.partial {
            let end = self.start + self.len - 1;
            put_header(
                &mut resp,
                header::CONTENT_RANGE,
                &format!("bytes {}-{end}/{}", self.start, self.total),
            )?;
        }
        Ok(resp)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges() {
        use RangeReq::*;
        assert_eq!(parse_range("bytes=0-99", 1000), Part(0, 99));
        assert_eq!(parse_range("bytes=500-", 1000), Part(500, 999));
        assert_eq!(parse_range("bytes=-100", 1000), Part(900, 999));
        assert_eq!(parse_range("bytes=-5000", 1000), Part(0, 999));
        assert_eq!(parse_range("bytes=990-5000", 1000), Part(990, 999));
        assert_eq!(parse_range("bytes=999-999", 1000), Part(999, 999));
        assert_eq!(parse_range("bytes=1000-", 1000), Unsatisfiable);
        assert_eq!(parse_range("bytes=1000-2000", 1000), Unsatisfiable);
        assert_eq!(parse_range("bytes=-0", 1000), Unsatisfiable);
        assert_eq!(parse_range("bytes=0-", 0), Unsatisfiable);
        assert_eq!(parse_range("bytes=-1", 0), Unsatisfiable);
        assert_eq!(parse_range("bytes=0-1,5-6", 1000), Full);
        assert_eq!(parse_range("bytes=5-1", 1000), Full);
        assert_eq!(parse_range("bytes=a-b", 1000), Full);
        assert_eq!(parse_range("bytes=-", 1000), Full);
        assert_eq!(parse_range("bytes=+1-2", 1000), Full);
        assert_eq!(parse_range("items=0-1", 1000), Full);
        assert_eq!(parse_range("bytes 0-1", 1000), Full);
        assert_eq!(
            parse_range("bytes=99999999999999999999999-", 1000),
            Unsatisfiable
        );
        assert_eq!(
            parse_range("bytes=0-99999999999999999999999", 1000),
            Part(0, 999)
        );
        assert_eq!(parse_range(" bytes= 1 - 2 ", 1000), Part(1, 2));
    }

    #[test]
    fn deferred_removal_waits_for_the_last_stream() {
        let dir = std::env::temp_dir().join(format!("od-streams-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("mkdir");
        let streams = Arc::new(Streams::default());
        assert!(!streams.defer_removal(5));
        let a = streams.begin(5, dir.clone());
        let b = streams.begin(5, dir.clone());
        assert!(streams.defer_removal(5));
        drop(a);
        assert!(dir.exists());
        drop(b); // no runtime here: removed right away
        assert!(!dir.exists());
        assert!(!streams.is_active(5));
    }
}
