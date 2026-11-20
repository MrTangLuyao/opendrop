//! Periodic cleanup: expired and used up parcels, stale uploads, expired
//! sessions, claims, accounts and code quarantines; hourly reconciliation
//! of the files dir.

use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime};

use rusqlite::{Connection, TransactionBehavior};
use tokio::task::JoinHandle;

use crate::App;
use crate::util::{AppError, now_ms};

/// Directories younger than this are never treated as orphans (a parcel row
/// may have been inserted after the sweep listed the live ids).
const ORPHAN_GRACE: Duration = Duration::from_secs(3600);
/// An unfinished upload with no data for this long is deleted. Bytes
/// arriving, and the end of every PUT that brought some, renew it; a PUT
/// that sends nothing does not.
pub const UPLOAD_STALE_MS: i64 = 15 * 60 * 1000;
/// Rows deleted per transaction, so the database is never held for long.
const BATCH: i64 = 500;
/// Orphan removal stops (and warns) when it would delete more than this
/// many directories and over a quarter of all: the database was probably
/// replaced or restored, and the files may still be wanted.
const ORPHAN_SAFE: usize = 16;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct SweepStats {
    pub parcels: usize,
    pub sessions: usize,
    pub claims: usize,
    pub accounts: usize,
    pub orphans: usize,
}

/// Up to `BATCH` parcels to delete, ids above `after_stale` for the stale
/// uploads (some of those may be skipped as in flight).
fn candidates(
    c: &Connection,
    now: i64,
    after_stale: i64,
) -> rusqlite::Result<(Vec<i64>, Vec<i64>)> {
    let mut ready = Vec::new();
    {
        let mut stmt = c.prepare_cached(
            "SELECT id FROM parcels WHERE state = 1 AND expires_at < ?1 LIMIT ?2",
        )?;
        for r in stmt.query_map((now, BATCH), |r| r.get(0))? {
            ready.push(r?);
        }
    }
    let room = BATCH - i64::try_from(ready.len()).unwrap_or(BATCH);
    if room > 0 {
        let mut stmt = c.prepare_cached(
            "SELECT id FROM parcels p WHERE state = 1 AND downloads_left = 0 \
             AND NOT EXISTS (SELECT 1 FROM claims c WHERE c.parcel_id = p.id AND c.expires_at > ?1) \
             LIMIT ?2",
        )?;
        for r in stmt.query_map((now, room), |r| r.get(0))? {
            ready.push(r?);
        }
    }
    let mut stale = Vec::new();
    let room = BATCH - i64::try_from(ready.len()).unwrap_or(BATCH);
    if room > 0 {
        let mut stmt = c.prepare_cached(
            "SELECT id FROM parcels WHERE state = 0 AND touched_at < ?1 AND id > ?2 \
             ORDER BY id LIMIT ?3",
        )?;
        let cutoff = now.saturating_sub(UPLOAD_STALE_MS);
        for r in stmt.query_map((cutoff, after_stale, room), |r| r.get(0))? {
            stale.push(r?);
        }
    }
    Ok((ready, stale))
}

/// Deletes up to `BATCH` rows matching `sql`'s inner select (bound to now).
fn delete_batch(c: &Connection, sql: &str, now: i64) -> rusqlite::Result<usize> {
    c.prepare_cached(sql)?.execute((now, BATCH))
}

/// One sweep. `full` also reconciles the files directory with the database.
pub async fn run(app: &Arc<App>, full: bool) -> Result<SweepStats, AppError> {
    let _single = app.sweep_lock.lock().await;
    let now = now_ms();
    let mut stats = SweepStats::default();
    let mut after_stale = 0i64;
    loop {
        let app2 = Arc::clone(app);
        let (ids, picked, last_stale) = app
            .db
            .call(move |c| {
                let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
                let (ready, stale) = candidates(&tx, now, after_stale)?;
                let picked = ready.len() + stale.len();
                let last_stale = stale.last().copied();
                let mut ids = Vec::with_capacity(picked);
                {
                    let mut del = tx.prepare_cached("DELETE FROM parcels WHERE id = ?1")?;
                    for id in ready {
                        del.execute([id])?;
                        ids.push(id);
                    }
                    for id in stale {
                        // uploads that are receiving bytes right now stay
                        if app2.inflight.is_active(id) {
                            continue;
                        }
                        del.execute([id])?;
                        ids.push(id);
                    }
                }
                tx.commit()?;
                Ok((ids, picked, last_stale))
            })
            .await?;
        stats.parcels += ids.len();
        remove_parcel_dirs(app, ids).await;
        if let Some(l) = last_stale {
            after_stale = l;
        }
        if i64::try_from(picked).unwrap_or(BATCH) < BATCH {
            break;
        }
    }
    let deletes = [
        "DELETE FROM sessions WHERE token_hash IN \
         (SELECT token_hash FROM sessions WHERE expires_at < ?1 LIMIT ?2)",
        "DELETE FROM claims WHERE token_hash IN \
         (SELECT token_hash FROM claims WHERE expires_at < ?1 LIMIT ?2)",
        "DELETE FROM accounts WHERE id IN (SELECT id FROM accounts WHERE is_admin = 0 \
         AND is_perm = 0 AND expires_at IS NOT NULL AND expires_at < ?1 LIMIT ?2)",
        "DELETE FROM retired_codes WHERE code IN \
         (SELECT code FROM retired_codes WHERE until < ?1 LIMIT ?2)",
    ];
    for (i, sql) in deletes.into_iter().enumerate() {
        loop {
            let n = app.db.call(move |c| Ok(delete_batch(c, sql, now)?)).await?;
            match i {
                0 => stats.sessions += n,
                1 => stats.claims += n,
                2 => stats.accounts += n,
                _ => {}
            }
            if i64::try_from(n).unwrap_or(BATCH) < BATCH {
                break;
            }
        }
    }
    if full {
        stats.orphans = reconcile(app).await?;
    }
    Ok(stats)
}

/// Deletes parcel directories on the blocking pool. A directory with a
/// download still streaming is left to the last stream to remove. Errors
/// are logged once per call.
pub async fn remove_parcel_dirs(app: &App, ids: Vec<i64>) {
    let dirs: Vec<_> = ids
        .into_iter()
        .filter(|&id| !app.streams.defer_removal(id))
        .map(|id| app.parcel_dir(id))
        .collect();
    if dirs.is_empty() {
        return;
    }
    let res = tokio::task::spawn_blocking(move || {
        let mut first_err = None;
        for d in dirs {
            match std::fs::remove_dir_all(&d) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound && first_err.is_none() => {
                    first_err = Some(format!("{}: {e}", d.display()));
                }
                _ => {}
            }
        }
        first_err
    })
    .await;
    match res {
        Ok(Some(e)) => crate::log!("warning: removing parcel files: {e}"),
        Ok(None) => {}
        Err(e) => crate::log!("warning: removing parcel files: {e}"),
    }
}

/// A directory name we created: a canonical positive integer.
fn dir_id(name: &std::ffi::OsStr) -> Option<i64> {
    name.to_str().and_then(|s| {
        s.parse::<i64>()
            .ok()
            .filter(|v| *v > 0 && v.to_string() == s)
    })
}

/// The largest parcel id that has a directory under `files_dir`.
pub fn highest_dir_id(files_dir: &Path) -> std::io::Result<i64> {
    let mut highest = 0;
    for entry in std::fs::read_dir(files_dir)? {
        if let Some(id) = dir_id(&entry?.file_name()) {
            highest = highest.max(id);
        }
    }
    Ok(highest)
}

/// Ids and states of every parcel, read in batches.
async fn live_ids(app: &Arc<App>) -> Result<(HashSet<i64>, HashSet<i64>), AppError> {
    let mut live = HashSet::new();
    let mut ready = HashSet::new();
    let mut after = 0i64;
    loop {
        let rows: Vec<(i64, i64)> = app
            .db
            .call(move |c| {
                let mut stmt = c.prepare_cached(
                    "SELECT id, state FROM parcels WHERE id > ?1 ORDER BY id LIMIT 10000",
                )?;
                let rows = stmt.query_map([after], |r| Ok((r.get(0)?, r.get(1)?)))?;
                let mut out = Vec::new();
                for r in rows {
                    out.push(r?);
                }
                Ok(out)
            })
            .await?;
        let Some(&(last, _)) = rows.last() else {
            break;
        };
        let full_page = rows.len() == 10_000;
        for (id, state) in rows {
            live.insert(id);
            if state == 1 {
                ready.insert(id);
            }
        }
        after = last;
        if !full_page {
            break;
        }
    }
    Ok((live, ready))
}

static MASS_WARNED: AtomicBool = AtomicBool::new(false);

/// Removes directories under files/ that belong to no parcel (after a grace
/// period) and leftover `.part` files of finished parcels.
async fn reconcile(app: &Arc<App>) -> Result<usize, AppError> {
    let (live, ready) = live_ids(app).await?;
    let files_dir = app.files_dir.clone();
    let app2 = Arc::clone(app);
    let res = tokio::task::spawn_blocking(move || -> std::io::Result<(usize, Option<String>)> {
        let now = SystemTime::now();
        let mut total = 0usize;
        let mut orphans = Vec::new();
        for entry in std::fs::read_dir(&files_dir)? {
            let entry = entry?;
            let Some(id) = dir_id(&entry.file_name()) else {
                continue; // only touch names we create
            };
            total += 1;
            let path = entry.path();
            if live.contains(&id) {
                let inner = if ready.contains(&id) {
                    std::fs::read_dir(&path).ok()
                } else {
                    None
                };
                for f in inner.into_iter().flatten().flatten() {
                    if f.file_name().to_str().is_some_and(|n| n.ends_with(".part")) {
                        let _ = std::fs::remove_file(f.path());
                    }
                }
                continue;
            }
            if app2.inflight.is_active(id) || app2.streams.is_active(id) {
                continue;
            }
            let old = entry
                .metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| now.duration_since(t).ok())
                .is_some_and(|age| age >= ORPHAN_GRACE);
            if old {
                orphans.push((path, entry.file_type().is_ok_and(|t| t.is_dir())));
            }
        }
        // A database without parcels next to old files, or a sweep that
        // would empty most of files/, means the database was replaced or
        // restored: keep the files and say so.
        let mass = !orphans.is_empty()
            && (live.is_empty() || (orphans.len() > ORPHAN_SAFE && orphans.len() * 4 > total));
        if mass {
            let msg = (!MASS_WARNED.swap(true, Ordering::Relaxed)).then(|| {
                format!(
                    "{} of {total} directories in {} belong to no parcel in the database; \
                     not removing them (delete them by hand if they are not needed)",
                    orphans.len(),
                    files_dir.display()
                )
            });
            return Ok((0, msg));
        }
        let mut removed = 0;
        let mut first_err = None;
        for (path, is_dir) in orphans {
            let r = if is_dir {
                std::fs::remove_dir_all(&path)
            } else {
                std::fs::remove_file(&path)
            };
            match r {
                Ok(()) => removed += 1,
                Err(e) => {
                    if first_err.is_none() {
                        first_err =
                            Some(format!("removing orphaned files: {}: {e}", path.display()));
                    }
                }
            }
        }
        Ok((removed, first_err))
    })
    .await;
    match res {
        Ok(Ok((n, msg))) => {
            if let Some(m) = msg {
                crate::log!("warning: {m}");
            }
            Ok(n)
        }
        Ok(Err(e)) => Err(AppError::internal("files dir", e)),
        Err(e) => Err(AppError::internal("sweep task", e)),
    }
}

/// Every 60 s a sweep; every hour a full one. Also prunes rate limiters and
/// frees idle read buffers.
pub fn spawn(app: Arc<App>) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(60));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        tick.tick().await; // the first tick is immediate; startup already swept
        let mut n: u64 = 0;
        loop {
            tick.tick().await;
            n = n.wrapping_add(1);
            let full = n.is_multiple_of(60);
            if let Err(e) = run(&app, full).await {
                crate::log!("warning: sweep failed: {}", e.code);
            }
            app.limits.prune();
            crate::body::trim_pool();
        }
    })
}
