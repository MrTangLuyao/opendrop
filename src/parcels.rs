//! Sending: create a parcel, stream files into it, finish or cancel; and the
//! signed in user's own parcels.

use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use http::{HeaderMap, StatusCode};
use http_body_util::BodyExt;
use hyper::body::Incoming;
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior};
use serde::Deserialize;
use serde_json::{Value, json};
use subtle::ConstantTimeEq;
use tokio::sync::{Notify, OwnedSemaphorePermit};
use tokio::task::JoinHandle;

use crate::App;
use crate::auth::{hash_password, require_session, session};
use crate::db::{self, opt};
use crate::disk;
use crate::http::{
    Ctx, HResult, JSON_LIMIT, Req, content_length, header_str, json_response, ok_json, read_json,
};
use crate::ratelimit::{Key, SlotGuard, tier_ranges};
use crate::settings::Settings;
use crate::sweep::remove_parcel_dirs;
use crate::util::{
    AppError, MIB, after_hours, dedup_names, hash_token, is_code, is_token, new_token, now_ms,
    random_code, sanitize_name,
};

pub const MAX_FILES: usize = 1000;
/// Defaults when the sender leaves the fields out (the web form uses the same).
pub const DEFAULT_DOWNLOADS: i64 = 20;
pub const DEFAULT_EXPIRY_HOURS: i64 = 48;
pub const MAX_TEXT_BYTES: usize = MIB as usize;
pub const MAX_DOWNLOADS: i64 = 999;
pub const MAX_PASSWORD_CHARS: usize = 128;
/// Body limit for POST /api/parcels: one MiB of text with room for JSON
/// escaping (newlines and quotes double), or 1000 file entries with long
/// paths.
pub const CREATE_LIMIT: usize = 2 * 1024 * 1024 + 64 * 1024;
/// Bytes charged against quota and disk admission for every file and for
/// every parcel on top of the file sizes: an empty file still costs an
/// inode, a directory entry and a block of metadata.
pub const ENTRY_OVERHEAD: u64 = 4096;
/// Unfinished (uploading) parcels one client may hold at once: per IPv4
/// address or IPv6 /64, /56 and /48. Each is a quota reservation.
pub const UPLOADING_PER_CLIENT: [i64; 3] = [3, 6, 12];
/// Unfinished parcels one signed in account may hold at once.
pub const UPLOADING_PER_ACCOUNT: i64 = 3;
/// How long a waiting large create body may queue for a buffer slot.
const BIG_BODY_WAIT: Duration = Duration::from_secs(10);
/// Retry-After when a client already has its maximum of uploads running.
const BUSY_RETRY_SECS: u64 = 5;
/// Received data is written in batches of about this size.
const WRITE_BATCH: usize = 1 << 20;
/// A partial batch is written after this much silence, so a stalled sender
/// holds no buffered data.
const IDLE_FLUSH: Duration = Duration::from_millis(200);
/// Attempts at a code that has not been used in the last week, before any
/// free code will do (the code space may be crowded).
const FRESH_CODE_TRIES: usize = 40;
const CODE_TRIES: usize = 50;

pub const KIND_FILES: i64 = 0;
pub const KIND_TEXT: i64 = 1;
pub const STATE_UPLOADING: i64 = 0;
pub const STATE_READY: i64 = 1;

pub fn kind_str(kind: i64) -> &'static str {
    if kind == KIND_TEXT { "text" } else { "files" }
}

fn size_mismatch() -> AppError {
    AppError::new(StatusCode::BAD_REQUEST, "size_mismatch")
}

fn too_large(s: &Settings) -> AppError {
    AppError::new(StatusCode::PAYLOAD_TOO_LARGE, "too_large").with("max", s.max_upload_bytes)
}

fn disk_full() -> AppError {
    AppError::new(StatusCode::INSUFFICIENT_STORAGE, "disk_full")
}

// ---------------------------------------------------------------------------
// In-flight uploads
// ---------------------------------------------------------------------------

struct Slot {
    generation: u64,
    cancel: Arc<Notify>,
    lock: Arc<tokio::sync::Mutex<()>>,
}

#[derive(Default)]
struct FlightInner {
    parcels: HashMap<i64, usize>,
    slots: HashMap<(i64, u32), Slot>,
    next_generation: u64,
}

/// Uploads currently receiving data. The sweeper never deletes a parcel
/// listed here, and a newer PUT of the same file takes over from an older
/// one (which is told to stop and is waited for).
#[derive(Default)]
pub struct InFlight {
    inner: Mutex<FlightInner>,
}

pub struct UploadGuard<'a> {
    flight: &'a InFlight,
    parcel: i64,
    idx: u32,
    generation: u64,
    pub cancel: Arc<Notify>,
    pub lock: Arc<tokio::sync::Mutex<()>>,
}

impl InFlight {
    fn inner(&self) -> std::sync::MutexGuard<'_, FlightInner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn begin(&self, parcel: i64, idx: u32) -> UploadGuard<'_> {
        let mut g = self.inner();
        *g.parcels.entry(parcel).or_insert(0) += 1;
        g.next_generation = g.next_generation.wrapping_add(1);
        let generation = g.next_generation;
        let cancel = Arc::new(Notify::new());
        let lock = match g.slots.get_mut(&(parcel, idx)) {
            Some(slot) => {
                slot.cancel.notify_one();
                slot.generation = generation;
                slot.cancel = Arc::clone(&cancel);
                Arc::clone(&slot.lock)
            }
            None => {
                let lock = Arc::new(tokio::sync::Mutex::new(()));
                g.slots.insert(
                    (parcel, idx),
                    Slot {
                        generation,
                        cancel: Arc::clone(&cancel),
                        lock: Arc::clone(&lock),
                    },
                );
                lock
            }
        };
        UploadGuard {
            flight: self,
            parcel,
            idx,
            generation,
            cancel,
            lock,
        }
    }

    pub fn is_current(&self, parcel: i64, idx: u32, generation: u64) -> bool {
        self.inner()
            .slots
            .get(&(parcel, idx))
            .is_some_and(|s| s.generation == generation)
    }

    pub fn is_active(&self, parcel: i64) -> bool {
        self.inner().parcels.contains_key(&parcel)
    }

    /// Tells every upload of `parcel` to stop (parcel deleted).
    pub fn cancel_parcel(&self, parcel: i64) {
        let g = self.inner();
        for ((p, _), slot) in &g.slots {
            if *p == parcel {
                slot.cancel.notify_one();
            }
        }
    }
}

impl Drop for UploadGuard<'_> {
    fn drop(&mut self) {
        let mut g = self.flight.inner();
        if let Some(n) = g.parcels.get_mut(&self.parcel) {
            *n = n.saturating_sub(1);
            if *n == 0 {
                g.parcels.remove(&self.parcel);
            }
        }
        if g.slots
            .get(&(self.parcel, self.idx))
            .is_some_and(|s| s.generation == self.generation)
        {
            g.slots.remove(&(self.parcel, self.idx));
        }
    }
}

// ---------------------------------------------------------------------------
// Create
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct FileDecl {
    name: String,
    size: u64,
}

#[derive(Deserialize)]
struct CreateReq {
    kind: String,
    #[serde(default)]
    files: Vec<FileDecl>,
    text: Option<String>,
    password: Option<String>,
    downloads: Option<i64>,
    expiry_hours: Option<i64>,
}

struct NewParcel {
    code_owner: Option<i64>,
    pass_hash: Option<String>,
    kind: i64,
    upload_hash: Option<[u8; 32]>,
    total: u64,
    /// `total` plus the per entry overhead.
    charged: u64,
    up_key: Key,
    downloads: i64,
    expiry_hours: i64,
    files: Vec<(String, u64)>,
}

fn too_many_uploads(max: i64) -> AppError {
    AppError::new(StatusCode::TOO_MANY_REQUESTS, "too_many_uploads").with("max", max)
}

/// Caps the unfinished parcels of one client (and its wider IPv6 networks)
/// and of one account. Each such parcel reserves quota without having sent
/// a byte, so without a cap a few requests could block everybody.
fn check_uploading(tx: &Connection, key: Key, account: Option<i64>) -> Result<(), AppError> {
    let mut by_key = tx.prepare_cached(
        "SELECT COUNT(*) FROM parcels WHERE state = 0 AND up_key BETWEEN ?1 AND ?2",
    )?;
    for (range, cap) in tier_ranges(key).into_iter().zip(UPLOADING_PER_CLIENT) {
        let Some((lo, hi)) = range else {
            continue;
        };
        let n: i64 =
            by_key.query_row((&lo.to_be_bytes()[..], &hi.to_be_bytes()[..]), |r| r.get(0))?;
        if n >= cap {
            return Err(too_many_uploads(cap));
        }
    }
    if let Some(a) = account {
        let n: i64 = tx
            .prepare_cached("SELECT COUNT(*) FROM parcels WHERE state = 0 AND account_id = ?1")?
            .query_row([a], |r| r.get(0))?;
        if n >= UPLOADING_PER_ACCOUNT {
            return Err(too_many_uploads(UPLOADING_PER_ACCOUNT));
        }
    }
    Ok(())
}

/// Inserts the parcel row with a fresh code. The first tries avoid codes
/// that ended within the last week (an old shared link must not open a
/// stranger's parcel); the last ones take any free code. `None` when every
/// try collided.
fn insert_parcel(
    tx: &Transaction<'_>,
    p: &NewParcel,
    now: i64,
    mut next_code: impl FnMut() -> Result<String, AppError>,
) -> Result<Option<(i64, String)>, AppError> {
    let total = i64::try_from(p.total).map_err(|_| AppError::bad_field("files"))?;
    let charged = i64::try_from(p.charged).map_err(|_| AppError::bad_field("files"))?;
    let up_key = p.up_key.to_be_bytes();
    let columns = "code, account_id, pass_hash, kind, state, upload_hash, total_bytes, \
                   downloads_left, expiry_hours, expires_at, created_at, touched_at, charged, up_key";
    let mut fresh = tx.prepare_cached(&format!(
        "INSERT INTO parcels({columns}) \
         SELECT ?1, ?2, ?3, ?4, 0, ?5, ?6, ?7, ?8, NULL, ?9, ?9, ?10, ?11 \
         WHERE NOT EXISTS (SELECT 1 FROM retired_codes WHERE code = ?1 AND until > ?9) \
         ON CONFLICT(code) DO NOTHING"
    ))?;
    let mut any = tx.prepare_cached(&format!(
        "INSERT INTO parcels({columns}) \
         VALUES (?1, ?2, ?3, ?4, 0, ?5, ?6, ?7, ?8, NULL, ?9, ?9, ?10, ?11) \
         ON CONFLICT(code) DO NOTHING"
    ))?;
    for attempt in 0..CODE_TRIES {
        let code = next_code()?;
        let params = rusqlite::params![
            code,
            p.code_owner,
            p.pass_hash,
            p.kind,
            p.upload_hash.as_ref().map(|h| &h[..]),
            total,
            p.downloads,
            p.expiry_hours,
            now,
            charged,
            &up_key[..],
        ];
        let stmt = if attempt < FRESH_CODE_TRIES {
            &mut fresh
        } else {
            &mut any
        };
        if stmt.execute(params)? == 1 {
            return Ok(Some((tx.last_insert_rowid(), code)));
        }
    }
    Ok(None)
}

/// Admission and the INSERT in one transaction under the database lock, so
/// concurrent creates cannot both pass the same check.
fn admit_and_insert(
    c: &mut Connection,
    data_dir: &Path,
    s: &Settings,
    p: &NewParcel,
    now: i64,
) -> Result<(i64, String), AppError> {
    let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
    if p.total > s.max_upload_bytes {
        return Err(too_large(s));
    }
    if p.kind == KIND_FILES {
        check_uploading(&tx, p.up_key, p.code_owner)?;
    }
    let d = disk::disk_info(data_dir).map_err(|e| AppError::internal("statvfs", e))?;
    let u = db::usage(&tx)?;
    let quota = disk::quota(s, d.total);
    if u.used.checked_add(p.charged).is_none_or(|t| t > quota) {
        return Err(AppError::new(
            StatusCode::INSUFFICIENT_STORAGE,
            "quota_full",
        ));
    }
    // Bytes promised to unfinished uploads are not on the disk yet but will
    // be: they count as taken, or the uploads together would eat the reserve.
    let need = p
        .charged
        .checked_add(disk::reserve(d.total))
        .and_then(|n| n.checked_add(u.pending_bytes));
    if need.is_none_or(|n| d.avail < n) {
        return Err(disk_full());
    }
    if d.inodes > 0 {
        let files = p.files.len() as u64 + 1; // and the directory
        let need = files
            .checked_add(u.pending_files)
            .and_then(|n| n.checked_add(disk::inode_reserve(d.inodes)));
        if need.is_none_or(|n| d.inodes_avail < n) {
            return Err(disk_full());
        }
    }
    let (id, code) = insert_parcel(&tx, p, now, random_code)?
        .ok_or_else(|| AppError::new(StatusCode::SERVICE_UNAVAILABLE, "busy"))?;
    {
        let mut stmt = tx.prepare_cached(
            "INSERT INTO files(parcel_id, idx, name, size, crc32, done) VALUES (?1, ?2, ?3, ?4, NULL, 0)",
        )?;
        for (i, (name, size)) in p.files.iter().enumerate() {
            let size = i64::try_from(*size).map_err(|_| too_large(s))?;
            stmt.execute((id, i as i64, name, size))?;
        }
    }
    tx.commit()?;
    Ok((id, code))
}

/// A buffer slot for a create body above 64 KiB: one of the client's few
/// and one of the server's `BIG_BODIES`, so that many slow senders cannot
/// make the server hold gigabytes of half received JSON.
async fn big_body_slot(
    app: &Arc<App>,
    ctx: &Ctx,
    headers: &HeaderMap,
) -> Result<Option<(SlotGuard, OwnedSemaphorePermit)>, AppError> {
    if content_length(headers).is_some_and(|n| n <= JSON_LIMIT as u64) {
        return Ok(None);
    }
    let mine = app
        .slots
        .bodies
        .try_acquire(ctx.ip)
        .ok_or_else(|| AppError::rate_limited(BUSY_RETRY_SECS))?;
    let permit = tokio::time::timeout(BIG_BODY_WAIT, Arc::clone(&app.body_sem).acquire_owned())
        .await
        .map_err(|_| AppError::busy(BUSY_RETRY_SECS))?
        .map_err(|e| AppError::internal("body semaphore", e))?;
    Ok(Some((mine, permit)))
}

pub async fn create(app: &Arc<App>, ctx: &Ctx, req: Req) -> HResult {
    app.limits
        .create
        .take(ctx.ip)
        .map_err(AppError::rate_limited)?;
    let sess = session(app, req.headers()).await?;
    let s = app.settings();
    // decided before a byte of the body is read
    if !s.anon_upload && sess.is_none() {
        return Err(AppError::new(StatusCode::UNAUTHORIZED, "login_required"));
    }
    let _buffer = big_body_slot(app, ctx, req.headers()).await?;
    let body: CreateReq = read_json(req, CREATE_LIMIT, false).await?;
    let downloads = body.downloads.unwrap_or(DEFAULT_DOWNLOADS);
    if !(1..=MAX_DOWNLOADS).contains(&downloads) {
        return Err(AppError::bad_field("downloads"));
    }
    let max_hours = i64::from(s.max_expiry_hours);
    let expiry_hours = body
        .expiry_hours
        .unwrap_or(DEFAULT_EXPIRY_HOURS.min(max_hours));
    if !(1..=max_hours).contains(&expiry_hours) {
        return Err(AppError::bad_field("expiry_hours"));
    }
    let password = body.password.unwrap_or_default();
    if password.chars().count() > MAX_PASSWORD_CHARS {
        return Err(AppError::bad_field("password"));
    }

    let (kind, files, text, total) = match body.kind.as_str() {
        "files" => {
            if body.files.is_empty() || body.files.len() > MAX_FILES {
                return Err(AppError::bad_field("files"));
            }
            let mut total: u64 = 0;
            for f in &body.files {
                total = total.checked_add(f.size).ok_or_else(|| too_large(&s))?;
            }
            if total > i64::MAX as u64 {
                return Err(too_large(&s));
            }
            let decls = body.files;
            // up to 1000 long names: string work for the blocking pool
            let files = tokio::task::spawn_blocking(move || {
                let names = dedup_names(decls.iter().map(|f| sanitize_name(&f.name)).collect());
                names
                    .into_iter()
                    .zip(decls.iter().map(|f| f.size))
                    .collect::<Vec<(String, u64)>>()
            })
            .await
            .map_err(|e| AppError::internal("names task", e))?;
            (KIND_FILES, files, None, total)
        }
        "text" => {
            let text = body.text.ok_or_else(|| AppError::bad_field("text"))?;
            if text.is_empty() || text.len() > MAX_TEXT_BYTES {
                return Err(AppError::bad_field("text"));
            }
            let len = text.len() as u64;
            (
                KIND_TEXT,
                vec![("text.txt".to_owned(), len)],
                Some(text),
                len,
            )
        }
        _ => return Err(AppError::bad_field("kind")),
    };
    // cheap precheck before spending an argon2 hash
    if total > s.max_upload_bytes {
        return Err(too_large(&s));
    }
    let entries = files.len() as u64 + 1;
    let charged = entries
        .checked_mul(ENTRY_OVERHEAD)
        .and_then(|o| o.checked_add(total))
        .ok_or_else(|| too_large(&s))?;
    let pass_hash = if password.is_empty() {
        None
    } else {
        Some(hash_password(app, password).await?)
    };
    let token = if kind == KIND_FILES {
        Some(new_token()?)
    } else {
        None
    };
    let new = NewParcel {
        code_owner: sess.map(|s| s.account_id),
        pass_hash,
        kind,
        upload_hash: token.as_ref().map(|t| t.hash),
        total,
        charged,
        up_key: ctx.ip,
        downloads,
        expiry_hours,
        files,
    };
    let now = now_ms();
    let data_dir = app.cfg.data_dir.clone();
    let (id, code) = app
        .db
        .call(move |c| admit_and_insert(c, &data_dir, &s, &new, now))
        .await?;

    match (token, text) {
        (Some(token), _) => Ok(json_response(
            StatusCode::CREATED,
            &json!({"id": id, "token": token.text}),
        )),
        (None, Some(text)) => store_text(app, id, &code, text, expiry_hours, downloads).await,
        (None, None) => Err(AppError::internal("create", "no token and no text")),
    }
}

/// Writes a text parcel's single file and marks the parcel ready.
async fn store_text(
    app: &Arc<App>,
    id: i64,
    code: &str,
    text: String,
    hours: i64,
    downloads: i64,
) -> HResult {
    let dir = app.parcel_dir(id);
    let written = tokio::task::spawn_blocking(move || -> io::Result<u32> {
        std::fs::create_dir_all(&dir)?;
        let part = dir.join("0.part");
        let mut f = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&part)?;
        f.write_all(text.as_bytes())?;
        f.sync_data()?;
        std::fs::rename(&part, dir.join("0"))?;
        Ok(crc32fast::hash(text.as_bytes()))
    })
    .await;
    let crc = match written {
        Ok(Ok(crc)) => crc,
        Ok(Err(e)) => {
            discard(app, id).await;
            return Err(AppError::io("store text", e));
        }
        Err(e) => {
            discard(app, id).await;
            return Err(AppError::internal("store text", e));
        }
    };
    let now = now_ms();
    let expires_at = after_hours(now, hours);
    app.db
        .call(move |c| {
            let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
            tx.execute(
                "UPDATE files SET done = 1, crc32 = ?1 WHERE parcel_id = ?2 AND idx = 0",
                (i64::from(crc), id),
            )?;
            tx.execute(
                "UPDATE parcels SET state = 1, expires_at = ?1, touched_at = ?2 WHERE id = ?3",
                (expires_at, now, id),
            )?;
            tx.commit()?;
            Ok(())
        })
        .await?;
    Ok(json_response(
        StatusCode::CREATED,
        &json!({"code": code, "expires_at": expires_at, "downloads_left": downloads}),
    ))
}

/// Deletes a parcel row and its directory (failure cleanup).
async fn discard(app: &Arc<App>, id: i64) {
    let _ = app
        .db
        .call(move |c| {
            c.execute("DELETE FROM parcels WHERE id = ?1", [id])?;
            Ok(())
        })
        .await;
    app.inflight.cancel_parcel(id);
    remove_parcel_dirs(app, vec![id]).await;
}

// ---------------------------------------------------------------------------
// Upload token checks
// ---------------------------------------------------------------------------

fn upload_token_hash(headers: &HeaderMap) -> Result<[u8; 32], AppError> {
    match header_str(headers, "x-upload-token") {
        Some(t) if is_token(t) => Ok(hash_token(t)),
        _ => Err(AppError::bad_token()),
    }
}

/// 404 unless the parcel exists and is uploading; 403 on a wrong token.
fn check_upload(c: &Connection, id: i64, token_hash: &[u8; 32]) -> Result<(), AppError> {
    let row: Option<(i64, Option<Vec<u8>>)> = opt(c
        .prepare_cached("SELECT state, upload_hash FROM parcels WHERE id = ?1")?
        .query_row([id], |r| Ok((r.get(0)?, r.get(1)?))))?;
    let Some((state, stored)) = row else {
        return Err(AppError::not_found());
    };
    if state != STATE_UPLOADING {
        return Err(AppError::not_found());
    }
    let stored = stored.unwrap_or_default();
    if stored.len() != 32 || !bool::from(stored.as_slice().ct_eq(&token_hash[..])) {
        return Err(AppError::bad_token());
    }
    Ok(())
}

struct FileSlot {
    size: u64,
    done: bool,
}

fn file_slot(
    c: &Connection,
    id: i64,
    idx: u32,
    token_hash: &[u8; 32],
) -> Result<FileSlot, AppError> {
    check_upload(c, id, token_hash)?;
    let row: Option<(i64, i64)> = c
        .prepare_cached("SELECT size, done FROM files WHERE parcel_id = ?1 AND idx = ?2")?
        .query_row((id, idx), |r| Ok((r.get(0)?, r.get(1)?)))
        .optional()?;
    let (size, done) = row.ok_or_else(AppError::not_found)?;
    Ok(FileSlot {
        size: u64::try_from(size).unwrap_or(0),
        done: done != 0,
    })
}

// ---------------------------------------------------------------------------
// PUT file
// ---------------------------------------------------------------------------

type WriteResult = io::Result<(File, crc32fast::Hasher, Vec<u8>)>;

/// Writes received data in batches on the blocking pool, one batch in
/// flight while the next one is being received. Frames are copied into two
/// reused buffers and dropped at once, so hyper can reuse its read buffer
/// instead of allocating a fresh one for every read.
struct Writer {
    state: Option<(File, crc32fast::Hasher)>,
    pending: Option<JoinHandle<WriteResult>>,
    batch: Vec<u8>,
    spare: Option<Vec<u8>>,
}

impl Writer {
    fn new(file: File) -> Self {
        Writer {
            state: Some((file, crc32fast::Hasher::new())),
            pending: None,
            batch: Vec::new(),
            spare: None,
        }
    }

    fn has_data(&self) -> bool {
        !self.batch.is_empty()
    }

    async fn wait(&mut self) -> Result<(), AppError> {
        if let Some(task) = self.pending.take() {
            let (f, h, buf) = task
                .await
                .map_err(|e| AppError::internal("write task", e))?
                .map_err(|e| AppError::io("write upload", e))?;
            self.state = Some((f, h));
            self.spare = Some(buf);
        }
        Ok(())
    }

    async fn flush(&mut self) -> Result<(), AppError> {
        if self.batch.is_empty() {
            return Ok(());
        }
        self.wait().await?;
        let (mut f, mut h) = self
            .state
            .take()
            .ok_or_else(|| AppError::internal("upload", "writer state missing"))?;
        let next = self.spare.take().unwrap_or_default();
        let mut buf = std::mem::replace(&mut self.batch, next);
        self.pending = Some(tokio::task::spawn_blocking(move || {
            h.update(&buf);
            f.write_all(&buf)?;
            buf.clear();
            Ok((f, h, buf))
        }));
        Ok(())
    }

    async fn push(&mut self, mut data: &[u8]) -> Result<(), AppError> {
        while !data.is_empty() {
            // the buffer grows with the data (a trickle holds little); the
            // spare comes back from the write at full size
            let room = WRITE_BATCH.saturating_sub(self.batch.len());
            let (now, rest) = data.split_at(room.min(data.len()));
            self.batch.extend_from_slice(now);
            data = rest;
            if self.batch.len() >= WRITE_BATCH {
                self.flush().await?;
            }
        }
        Ok(())
    }

    /// Writes what is buffered and frees both buffers (the sender went
    /// quiet; a stalled upload holds no memory).
    async fn idle(&mut self) -> Result<(), AppError> {
        self.flush().await?;
        self.wait().await?;
        self.batch = Vec::new();
        self.spare = None;
        Ok(())
    }

    async fn finish(mut self) -> Result<u32, AppError> {
        self.flush().await?;
        self.wait().await?;
        let (f, h) = self
            .state
            .take()
            .ok_or_else(|| AppError::internal("upload", "writer state missing"))?;
        tokio::task::spawn_blocking(move || f.sync_data())
            .await
            .map_err(|e| AppError::internal("sync task", e))?
            .map_err(|e| AppError::io("sync upload", e))?;
        Ok(h.finalize())
    }
}

/// Marks an uploading parcel as active now.
async fn touch(app: &Arc<App>, id: i64) {
    let now = now_ms();
    let _ = app
        .db
        .call(move |c| {
            c.execute(
                "UPDATE parcels SET touched_at = ?1 WHERE id = ?2 AND state = 0",
                (now, id),
            )?;
            Ok(())
        })
        .await;
}

/// Opens `part` for `size` bytes. The space was admitted with the parcel;
/// if the disk has filled up from elsewhere since, the reserve is not given
/// away now either.
fn open_part(dir: &Path, part: &Path, size: u64) -> io::Result<File> {
    std::fs::create_dir_all(dir)?;
    if size > 0
        && let Ok(d) = disk::disk_info(dir)
        && d.avail < size.saturating_add(disk::reserve(d.total))
    {
        return Err(io::Error::from(io::ErrorKind::StorageFull));
    }
    let f = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(part)?;
    disk::preallocate(&f, size)?;
    Ok(f)
}

/// Streams the body into `part`; returns the CRC32 of exactly `size` bytes.
async fn receive(
    app: &Arc<App>,
    id: i64,
    mut body: Incoming,
    cancel: &Notify,
    part: PathBuf,
    size: u64,
) -> Result<u32, AppError> {
    let mut received: u64 = 0;
    let dir = app.parcel_dir(id);
    let file = tokio::task::spawn_blocking(move || open_part(&dir, &part, size))
        .await
        .map_err(|e| AppError::internal("open task", e))?
        .map_err(|e| AppError::io("open upload", e))?;

    let pace = app.cfg.pace;
    let mut writer = Writer::new(file);
    let mut last_data = Instant::now();
    // The throughput floor: every window must bring a minimum of bytes.
    // Each window that does renews touched_at, so a long PUT never looks
    // stale, while a trickle renews nothing.
    let mut window_start = Instant::now();
    let mut window_base: u64 = 0;
    loop {
        let now = Instant::now();
        let wait = if writer.has_data() {
            IDLE_FLUSH
        } else {
            let idle_left = pace
                .upload_idle
                .saturating_sub(now.saturating_duration_since(last_data));
            let window_left = pace
                .upload_window
                .saturating_sub(now.saturating_duration_since(window_start));
            idle_left.min(window_left)
        };
        let next = tokio::select! {
            biased;
            () = cancel.notified() => return Err(AppError::new(StatusCode::CONFLICT, "superseded")),
            r = tokio::time::timeout(wait, body.frame()) => r,
        };
        match next {
            Err(_) => {
                if writer.has_data() {
                    writer.idle().await?;
                }
            }
            Ok(None) => break,
            Ok(Some(Err(_))) => return Err(AppError::new(StatusCode::BAD_REQUEST, "aborted")),
            Ok(Some(Ok(frame))) => {
                let Ok(data) = frame.into_data() else {
                    continue; // trailers carry nothing we need
                };
                if !data.is_empty() {
                    last_data = Instant::now();
                }
                received = received
                    .checked_add(data.len() as u64)
                    .ok_or_else(size_mismatch)?;
                if received > size {
                    return Err(size_mismatch());
                }
                writer.push(&data).await?;
            }
        }
        let now = Instant::now();
        if now.saturating_duration_since(last_data) >= pace.upload_idle {
            return Err(AppError::new(StatusCode::REQUEST_TIMEOUT, "timeout"));
        }
        if now.saturating_duration_since(window_start) >= pace.upload_window {
            if received.saturating_sub(window_base) < pace.upload_min_bytes {
                return Err(AppError::new(StatusCode::REQUEST_TIMEOUT, "too_slow"));
            }
            touch(app, id).await;
            window_start = now;
            window_base = received;
        }
    }
    if received != size {
        return Err(size_mismatch());
    }
    writer.finish().await
}

pub async fn put_file(app: &Arc<App>, ctx: &Ctx, req: Req, id: i64, idx: u32) -> HResult {
    let th = upload_token_hash(req.headers())?;
    let declared = content_length(req.headers());
    let slot = app.db.call(move |c| file_slot(c, id, idx, &th)).await?;
    if slot.done {
        return Err(AppError::new(StatusCode::CONFLICT, "already_done"));
    }
    if declared.is_some_and(|n| n != slot.size) {
        return Err(size_mismatch());
    }

    // Only a holder of the token gets here, so only they can take over.
    let guard = app.inflight.begin(id, idx);
    let _lock = Arc::clone(&guard.lock).lock_owned().await;
    if !app.inflight.is_current(id, idx, guard.generation) {
        return Err(AppError::new(StatusCode::CONFLICT, "superseded"));
    }
    // taken after an older upload of this file has let go of its own
    let _client = app
        .slots
        .uploads
        .try_acquire(ctx.ip)
        .ok_or_else(|| AppError::rate_limited(BUSY_RETRY_SECS))?;
    // No touched_at here: a PUT that sends no data must not keep a parcel
    // (and its quota reservation) alive.
    let slot = app.db.call(move |c| file_slot(c, id, idx, &th)).await?;
    if slot.done {
        return Err(AppError::new(StatusCode::CONFLICT, "already_done"));
    }

    let dir = app.parcel_dir(id);
    let part = dir.join(format!("{idx}.part"));
    let fin = dir.join(idx.to_string());
    // touched_at is renewed while the data flows (see receive), so when a
    // long PUT breaks off the retry still has the whole stale grace
    let result = receive(
        app,
        id,
        req.into_body(),
        &guard.cancel,
        part.clone(),
        slot.size,
    )
    .await;
    let crc = match result {
        Ok(crc) => crc,
        Err(e) => {
            let _ = tokio::task::spawn_blocking(move || std::fs::remove_file(part)).await;
            return Err(e);
        }
    };
    let (p2, f2) = (part.clone(), fin.clone());
    match tokio::task::spawn_blocking(move || std::fs::rename(p2, f2)).await {
        Ok(Ok(())) => {}
        Ok(Err(e)) if e.kind() == io::ErrorKind::NotFound => return Err(AppError::not_found()),
        Ok(Err(e)) => return Err(AppError::io("rename upload", e)),
        Err(e) => return Err(AppError::internal("rename task", e)),
    }
    let now = now_ms();
    let updated = app
        .db
        .call(move |c| {
            let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let n = tx.execute(
                "UPDATE files SET done = 1, crc32 = ?1 WHERE parcel_id = ?2 AND idx = ?3 AND done = 0 \
                 AND EXISTS (SELECT 1 FROM parcels WHERE id = ?2 AND state = 0)",
                (i64::from(crc), id, idx),
            )?;
            tx.execute("UPDATE parcels SET touched_at = ?1 WHERE id = ?2", (now, id))?;
            tx.commit()?;
            Ok(n)
        })
        .await?;
    if updated == 0 {
        // the parcel went away while we were writing
        let _ = tokio::task::spawn_blocking(move || std::fs::remove_file(fin)).await;
        return Err(AppError::not_found());
    }
    let mut resp = http::Response::new(crate::body::Body::Empty);
    *resp.status_mut() = StatusCode::NO_CONTENT;
    Ok(resp)
}

// ---------------------------------------------------------------------------
// Finish / cancel
// ---------------------------------------------------------------------------

pub async fn finish(app: &Arc<App>, req: &Req, id: i64) -> HResult {
    let th = upload_token_hash(req.headers())?;
    let now = now_ms();
    let v = app
        .db
        .call(move |c| {
            let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
            check_upload(&tx, id, &th)?;
            let (count, missing): (i64, i64) = tx.query_row(
                "SELECT COUNT(*), COALESCE(SUM(done = 0), 0) FROM files WHERE parcel_id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            if missing > 0 {
                return Err(
                    AppError::new(StatusCode::CONFLICT, "incomplete").with("missing", missing)
                );
            }
            let (code, expires_at, left, total): (String, i64, Option<i64>, i64) = tx.query_row(
                "UPDATE parcels SET state = 1, upload_hash = NULL, \
                 expires_at = ?2 + expiry_hours * 3600000, touched_at = ?2 \
                 WHERE id = ?1 RETURNING code, expires_at, downloads_left, total_bytes",
                (id, now),
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )?;
            tx.commit()?;
            Ok(json!({
                "code": code,
                "expires_at": expires_at,
                "downloads_left": left,
                "total_bytes": total,
                "file_count": count,
            }))
        })
        .await?;
    ok_json(v)
}

pub async fn cancel(app: &Arc<App>, req: &Req, id: i64) -> HResult {
    let th = upload_token_hash(req.headers())?;
    app.db
        .call(move |c| {
            let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
            check_upload(&tx, id, &th)?;
            tx.execute("DELETE FROM parcels WHERE id = ?1", [id])?;
            tx.commit()?;
            Ok(())
        })
        .await?;
    app.inflight.cancel_parcel(id);
    remove_parcel_dirs(app, vec![id]).await;
    ok_json(json!({"ok": true}))
}

// ---------------------------------------------------------------------------
// My parcels
// ---------------------------------------------------------------------------

const MINE_COLUMNS: &str = "p.code, p.kind, (SELECT COUNT(*) FROM files f WHERE f.parcel_id = p.id), \
     p.total_bytes, p.created_at, p.expires_at, p.downloads_left, p.pass_hash IS NOT NULL";

fn mine_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Value> {
    Ok(json!({
        "code": r.get::<_, String>(0)?,
        "kind": kind_str(r.get(1)?),
        "file_count": r.get::<_, i64>(2)?,
        "total_bytes": r.get::<_, i64>(3)?,
        "created_at": r.get::<_, i64>(4)?,
        "expires_at": r.get::<_, Option<i64>>(5)?,
        "downloads_left": r.get::<_, Option<i64>>(6)?,
        "has_password": r.get::<_, bool>(7)?,
    }))
}

pub async fn mine(app: &Arc<App>, req: &Req) -> HResult {
    let s = require_session(app, req.headers()).await?;
    let id = s.account_id;
    let rows = app
        .db
        .call(move |c| {
            let mut stmt = c.prepare_cached(&format!(
                "SELECT {MINE_COLUMNS} FROM parcels p WHERE p.account_id = ?1 AND p.state = 1 \
                 ORDER BY p.created_at DESC, p.id DESC"
            ))?;
            let rows = stmt.query_map([id], mine_row)?;
            let mut out = Vec::new();
            for r in rows {
                out.push(r?);
            }
            Ok(out)
        })
        .await?;
    ok_json(json!({"parcels": rows}))
}

/// Parcel id of a ready parcel by code, checking ownership.
async fn owned_parcel(app: &Arc<App>, account: i64, code: &str) -> Result<i64, AppError> {
    if !is_code(code) {
        return Err(AppError::new(StatusCode::BAD_REQUEST, "bad_code"));
    }
    let code = code.to_owned();
    let row: Option<(i64, Option<i64>)> = app
        .db
        .call(move |c| {
            opt(c
                .prepare_cached("SELECT id, account_id FROM parcels WHERE code = ?1 AND state = 1")?
                .query_row([&code], |r| Ok((r.get(0)?, r.get(1)?))))
        })
        .await?;
    let (id, owner) = row.ok_or_else(AppError::not_found)?;
    if owner != Some(account) {
        return Err(AppError::new(StatusCode::FORBIDDEN, "not_owner"));
    }
    Ok(id)
}

#[derive(Deserialize)]
struct MinePatch {
    expiry_hours: Option<i64>,
    downloads: Option<i64>,
    password: Option<String>,
}

pub async fn mine_patch(app: &Arc<App>, req: Req, code: &str) -> HResult {
    let s = require_session(app, req.headers()).await?;
    let id = owned_parcel(app, s.account_id, code).await?;
    let body: MinePatch = read_json(req, JSON_LIMIT, false).await?;
    let max_hours = i64::from(app.settings().max_expiry_hours);
    if body
        .expiry_hours
        .is_some_and(|h| !(1..=max_hours).contains(&h))
    {
        return Err(AppError::bad_field("expiry_hours"));
    }
    if body
        .downloads
        .is_some_and(|d| !(1..=MAX_DOWNLOADS).contains(&d))
    {
        return Err(AppError::bad_field("downloads"));
    }
    let password = match body.password {
        None => PasswordChange::Keep,
        Some(p) if p.is_empty() => PasswordChange::Clear,
        Some(p) => {
            if p.chars().count() > MAX_PASSWORD_CHARS {
                return Err(AppError::bad_field("password"));
            }
            PasswordChange::Set(hash_password(app, p).await?)
        }
    };
    let account = s.account_id;
    let now = now_ms();
    let (hours, downloads) = (body.expiry_hours, body.downloads);
    let row = app
        .db
        .call(move |c| {
            let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let owner: Option<Option<i64>> = opt(tx.query_row(
                "SELECT account_id FROM parcels WHERE id = ?1 AND state = 1",
                [id],
                |r| r.get(0),
            ))?;
            match owner {
                None => return Err(AppError::not_found()),
                Some(o) if o != Some(account) => {
                    return Err(AppError::new(StatusCode::FORBIDDEN, "not_owner"));
                }
                Some(_) => {}
            }
            if let Some(h) = hours {
                tx.execute(
                    "UPDATE parcels SET expires_at = ?1, expiry_hours = ?2 WHERE id = ?3",
                    (after_hours(now, h), h, id),
                )?;
            }
            if let Some(d) = downloads {
                tx.execute(
                    "UPDATE parcels SET downloads_left = ?1 WHERE id = ?2",
                    (d, id),
                )?;
            }
            password.apply(&tx, id)?;
            let row = tx.query_row(
                &format!("SELECT {MINE_COLUMNS} FROM parcels p WHERE p.id = ?1"),
                [id],
                mine_row,
            )?;
            tx.commit()?;
            Ok(row)
        })
        .await?;
    ok_json(row)
}

pub async fn mine_delete(app: &Arc<App>, req: &Req, code: &str) -> HResult {
    let s = require_session(app, req.headers()).await?;
    let id = owned_parcel(app, s.account_id, code).await?;
    let account = s.account_id;
    let n = app
        .db
        .call(move |c| {
            Ok(c.execute(
                "DELETE FROM parcels WHERE id = ?1 AND account_id = ?2",
                (id, account),
            )?)
        })
        .await?;
    if n == 0 {
        return Err(AppError::not_found());
    }
    remove_parcel_dirs(app, vec![id]).await;
    ok_json(json!({"ok": true}))
}

/// Three state password edit: absent keeps, empty clears, value replaces.
pub enum PasswordChange {
    Keep,
    Clear,
    Set(String),
}

impl PasswordChange {
    pub fn apply(&self, c: &Connection, id: i64) -> Result<(), AppError> {
        match self {
            PasswordChange::Keep => {}
            PasswordChange::Clear => {
                c.execute("UPDATE parcels SET pass_hash = NULL WHERE id = ?1", [id])?;
            }
            PasswordChange::Set(h) => {
                c.execute("UPDATE parcels SET pass_hash = ?1 WHERE id = ?2", (h, id))?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn inflight_takeover() {
        let f = InFlight::default();
        let a = f.begin(7, 0);
        assert!(f.is_active(7));
        assert!(f.is_current(7, 0, a.generation));
        let lock_a = Arc::clone(&a.lock).lock_owned().await;
        let b = f.begin(7, 0);
        // the older upload was told to stop and is no longer current
        assert!(!f.is_current(7, 0, a.generation));
        assert!(f.is_current(7, 0, b.generation));
        tokio::time::timeout(Duration::from_secs(1), a.cancel.notified())
            .await
            .expect("cancel delivered");
        // the newer one waits for the older one's lock
        let lb = Arc::clone(&b.lock);
        assert!(lb.try_lock().is_err());
        drop(lock_a);
        drop(a);
        assert!(lb.try_lock().is_ok());
        assert!(f.is_current(7, 0, b.generation));
        drop(b);
        assert!(!f.is_active(7));
        assert!(f.inner().slots.is_empty());
    }

    #[test]
    fn codes_that_ended_recently_are_avoided() {
        let dir = std::env::temp_dir().join(format!("od-codes-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir");
        let db = crate::db::Db::open(&dir.join("t.db")).expect("db");
        let p = NewParcel {
            code_owner: None,
            pass_hash: None,
            kind: KIND_FILES,
            upload_hash: None,
            total: 1,
            charged: 1,
            up_key: 1,
            downloads: 1,
            expiry_hours: 1,
            files: Vec::new(),
        };
        let now = now_ms();
        db.blocking(|c| {
            c.execute(
                "INSERT INTO retired_codes(code, until) VALUES ('111111', ?1), ('333333', 1)",
                [now + 1000],
            )?;
            let tx = c.transaction()?;
            let code = |r: Option<(i64, String)>| r.map(|(_, c)| c);
            // a code still in quarantine is passed over
            let mut codes = ["111111", "222222"].into_iter().map(String::from);
            let got = insert_parcel(&tx, &p, now, || Ok(codes.next().unwrap_or_default()))?;
            assert_eq!(code(got).as_deref(), Some("222222"));
            // an expired quarantine does not count
            let got = insert_parcel(&tx, &p, now, || Ok("333333".to_owned()))?;
            assert_eq!(code(got).as_deref(), Some("333333"));
            // when only quarantined codes come up, the last tries take one
            let got = insert_parcel(&tx, &p, now, || Ok("111111".to_owned()))?;
            assert_eq!(code(got).as_deref(), Some("111111"));
            // and when every try collides there is none
            assert!(insert_parcel(&tx, &p, now, || Ok("222222".to_owned()))?.is_none());
            Ok(())
        })
        .expect("db");
        drop(db);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
