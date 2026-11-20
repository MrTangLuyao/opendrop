//! SQLite access: one connection behind a mutex, every call on the blocking
//! pool, the lock held only for the closure.

use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use rusqlite::{Connection, OpenFlags};

use crate::parcels::ENTRY_OVERHEAD;
use crate::util::{AppError, DAY_MS};

pub const SCHEMA_VERSION: i64 = 2;

/// How long the code of an ended parcel stays out of use, so that an old
/// shared link does not open a stranger's new parcel.
pub const RETIRED_CODE_MS: i64 = 7 * DAY_MS;

/// Tables and indexes of version 1.
const SCHEMA_V1: &str = "
CREATE TABLE IF NOT EXISTS accounts(
    id INTEGER PRIMARY KEY,
    username TEXT NOT NULL UNIQUE COLLATE NOCASE,
    pass_hash TEXT NOT NULL,
    is_admin INTEGER NOT NULL DEFAULT 0,
    is_perm INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL,
    expires_at INTEGER
);
CREATE TABLE IF NOT EXISTS sessions(
    token_hash BLOB PRIMARY KEY,
    account_id INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    expires_at INTEGER NOT NULL
) WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS parcels(
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    code TEXT NOT NULL UNIQUE,
    account_id INTEGER REFERENCES accounts(id) ON DELETE SET NULL,
    pass_hash TEXT,
    kind INTEGER NOT NULL,
    state INTEGER NOT NULL,
    upload_hash BLOB,
    total_bytes INTEGER NOT NULL,
    downloads_left INTEGER,
    expiry_hours INTEGER NOT NULL,
    expires_at INTEGER,
    created_at INTEGER NOT NULL,
    touched_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS files(
    parcel_id INTEGER NOT NULL REFERENCES parcels(id) ON DELETE CASCADE,
    idx INTEGER NOT NULL,
    name TEXT NOT NULL,
    size INTEGER NOT NULL,
    crc32 INTEGER,
    done INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY(parcel_id, idx)
) WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS claims(
    token_hash BLOB PRIMARY KEY,
    parcel_id INTEGER NOT NULL REFERENCES parcels(id) ON DELETE CASCADE,
    expires_at INTEGER NOT NULL
) WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS settings(
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
) WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS parcels_account ON parcels(account_id);
CREATE INDEX IF NOT EXISTS parcels_state_expires ON parcels(state, expires_at);
CREATE INDEX IF NOT EXISTS sessions_expires ON sessions(expires_at);
CREATE INDEX IF NOT EXISTS sessions_account ON sessions(account_id);
CREATE INDEX IF NOT EXISTS claims_parcel ON claims(parcel_id);
CREATE INDEX IF NOT EXISTS claims_expires ON claims(expires_at);
";

/// Version 2 adds, to parcels: `charged` (bytes counted against quota and
/// disk: the sizes plus `ENTRY_OVERHEAD` per file and per parcel) and
/// `up_key` (the creator's client key, to cap unfinished uploads per
/// client). Each statement runs once, when the database moves to v2.
fn migrate_v2() -> String {
    format!(
        "ALTER TABLE parcels ADD COLUMN charged INTEGER NOT NULL DEFAULT 0;
ALTER TABLE parcels ADD COLUMN up_key BLOB;
UPDATE parcels SET charged = total_bytes + {ENTRY_OVERHEAD} * \
(1 + (SELECT COUNT(*) FROM files f WHERE f.parcel_id = parcels.id));
"
    )
}

/// Tables, indexes and triggers of version 2 (all idempotent).
///
/// `usage` holds running totals kept by triggers in the same transaction as
/// every insert and delete, so no request has to sum the parcels table:
/// `used` (charged bytes of all parcels), `parcels` (row count), and the
/// bytes and files that unfinished uploads have been promised but not yet
/// written (`pending_*`, files with done = 0). Foreign key cascades fire the
/// files triggers too. A ready parcel's code goes to `retired_codes` when the
/// row is deleted.
fn schema_v2() -> String {
    format!(
        "
CREATE TABLE IF NOT EXISTS usage(
    id INTEGER PRIMARY KEY CHECK (id = 1),
    used INTEGER NOT NULL,
    parcels INTEGER NOT NULL,
    pending_bytes INTEGER NOT NULL,
    pending_files INTEGER NOT NULL
);
INSERT OR IGNORE INTO usage(id, used, parcels, pending_bytes, pending_files) VALUES (1, 0, 0, 0, 0);
CREATE TABLE IF NOT EXISTS retired_codes(
    code TEXT PRIMARY KEY,
    until INTEGER NOT NULL
) WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS retired_until ON retired_codes(until);
CREATE INDEX IF NOT EXISTS parcels_uploading ON parcels(up_key) WHERE state = 0;
CREATE INDEX IF NOT EXISTS parcels_used_up ON parcels(state) WHERE downloads_left = 0;
CREATE TRIGGER IF NOT EXISTS parcels_count_ins AFTER INSERT ON parcels BEGIN
    UPDATE usage SET used = used + NEW.charged, parcels = parcels + 1 WHERE id = 1;
END;
CREATE TRIGGER IF NOT EXISTS parcels_count_del AFTER DELETE ON parcels BEGIN
    UPDATE usage SET used = used - OLD.charged, parcels = parcels - 1 WHERE id = 1;
    INSERT INTO retired_codes(code, until)
        SELECT OLD.code, CAST((julianday('now') - 2440587.5) * 86400000.0 AS INTEGER) + {RETIRED_CODE_MS}
        WHERE OLD.state = 1
        ON CONFLICT(code) DO UPDATE SET until = excluded.until;
END;
CREATE TRIGGER IF NOT EXISTS files_pending_ins AFTER INSERT ON files WHEN NEW.done = 0 BEGIN
    UPDATE usage SET pending_bytes = pending_bytes + NEW.size, pending_files = pending_files + 1 WHERE id = 1;
END;
CREATE TRIGGER IF NOT EXISTS files_pending_done AFTER UPDATE OF done ON files
    WHEN OLD.done = 0 AND NEW.done != 0 BEGIN
    UPDATE usage SET pending_bytes = pending_bytes - OLD.size, pending_files = pending_files - 1 WHERE id = 1;
END;
CREATE TRIGGER IF NOT EXISTS files_pending_del AFTER DELETE ON files WHEN OLD.done = 0 BEGIN
    UPDATE usage SET pending_bytes = pending_bytes - OLD.size, pending_files = pending_files - 1 WHERE id = 1;
END;
"
    )
}

const TABLES_V1: [&str; 6] = [
    "accounts", "sessions", "parcels", "files", "claims", "settings",
];
const TABLES_V2: [&str; 2] = ["usage", "retired_codes"];

/// Running totals (see [`schema_v2`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Usage {
    pub used: u64,
    pub parcels: u64,
    pub pending_bytes: u64,
    pub pending_files: u64,
}

pub fn usage(c: &Connection) -> Result<Usage, AppError> {
    let u = |v: i64| u64::try_from(v).unwrap_or(0);
    Ok(c.prepare_cached(
        "SELECT used, parcels, pending_bytes, pending_files FROM usage WHERE id = 1",
    )?
    .query_row([], |r| {
        Ok(Usage {
            used: u(r.get(0)?),
            parcels: u(r.get(1)?),
            pending_bytes: u(r.get(2)?),
            pending_files: u(r.get(3)?),
        })
    })?)
}

/// Recomputes the running totals from the tables (at startup, so that they
/// can never drift for long).
fn refresh_usage(c: &Connection) -> rusqlite::Result<()> {
    c.execute(
        "UPDATE usage SET used = (SELECT COALESCE(SUM(charged), 0) FROM parcels), \
         parcels = (SELECT COUNT(*) FROM parcels), \
         pending_bytes = (SELECT COALESCE(SUM(size), 0) FROM files WHERE done = 0), \
         pending_files = (SELECT COUNT(*) FROM files WHERE done = 0) WHERE id = 1",
        [],
    )?;
    Ok(())
}

/// Makes sure new parcel ids are above `highest` (the largest directory name
/// found under files/).
pub fn reserve_ids(c: &mut Connection, highest: i64) -> Result<(), AppError> {
    if highest <= 0 {
        return Ok(());
    }
    let tx = c.transaction()?;
    let seq: Option<i64> = opt(tx.query_row(
        "SELECT seq FROM sqlite_sequence WHERE name = 'parcels'",
        [],
        |r| r.get(0),
    ))?;
    match seq {
        None => {
            tx.execute(
                "INSERT INTO sqlite_sequence(name, seq) VALUES ('parcels', ?1)",
                [highest],
            )?;
        }
        Some(s) if s < highest => {
            tx.execute(
                "UPDATE sqlite_sequence SET seq = ?1 WHERE name = 'parcels'",
                [highest],
            )?;
        }
        Some(_) => {}
    }
    tx.commit()?;
    Ok(())
}

#[derive(Clone)]
pub struct Db {
    conn: Arc<Mutex<Connection>>,
}

impl Db {
    /// Opens (or creates) the database. A file that exists but cannot be
    /// opened, fails the integrity check or has an unknown schema is an
    /// error: the server must never start on top of a damaged store.
    pub fn open(path: &Path) -> Result<Db, String> {
        let shown = path.display();
        let conn = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_CREATE
                | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(|e| format!("cannot open database {shown}: {e}"))?;
        init(&conn).map_err(|e| format!("database {shown}: {e}"))?;
        Ok(Db {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// Runs `f` with the connection on the blocking pool.
    pub async fn call<R, F>(&self, f: F) -> Result<R, AppError>
    where
        F: FnOnce(&mut Connection) -> Result<R, AppError> + Send + 'static,
        R: Send + 'static,
    {
        let conn = Arc::clone(&self.conn);
        match tokio::task::spawn_blocking(move || {
            let mut guard = conn.lock().unwrap_or_else(PoisonError::into_inner);
            f(&mut guard)
        })
        .await
        {
            Ok(r) => r,
            Err(e) => Err(AppError::internal("database task", e)),
        }
    }

    /// Same as [`Db::call`] for synchronous callers (startup and CLI).
    pub fn blocking<R>(
        &self,
        f: impl FnOnce(&mut Connection) -> Result<R, AppError>,
    ) -> Result<R, AppError> {
        let mut guard = self.conn.lock().unwrap_or_else(PoisonError::into_inner);
        f(&mut guard)
    }
}

fn init(conn: &Connection) -> Result<(), String> {
    let e = |e: rusqlite::Error| e.to_string();
    conn.busy_timeout(Duration::from_secs(5)).map_err(e)?;
    let mode: String = conn
        .query_row("PRAGMA journal_mode=WAL", [], |r| r.get(0))
        .map_err(e)?;
    if !mode.eq_ignore_ascii_case("wal") {
        crate::log!("warning: database journal mode is {mode}, not wal");
    }
    conn.pragma_update(None, "synchronous", "NORMAL")
        .map_err(e)?;
    conn.pragma_update(None, "foreign_keys", "ON").map_err(e)?;
    // The container root is read only: never let SQLite reach for a temp dir.
    conn.pragma_update(None, "temp_store", "MEMORY")
        .map_err(e)?;

    let check: String = conn
        .query_row("PRAGMA quick_check", [], |r| r.get(0))
        .map_err(e)?;
    if check != "ok" {
        return Err(format!("integrity check failed: {check}"));
    }

    let version: i64 = conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .map_err(e)?;
    let require = |tables: &[&str]| -> Result<(), String> {
        for t in tables {
            let n: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
                    [t],
                    |r| r.get(0),
                )
                .map_err(e)?;
            if n != 1 {
                return Err(format!("table {t} is missing; refusing to start"));
            }
        }
        Ok(())
    };
    match version {
        0 | 1 => {
            if version == 0 {
                let tables: i64 = conn
                    .query_row(
                        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'",
                        [],
                        |r| r.get(0),
                    )
                    .map_err(e)?;
                if tables > 0 {
                    return Err(
                        "unknown schema (user_version 0 but tables exist); refusing to start"
                            .into(),
                    );
                }
            } else {
                require(&TABLES_V1)?;
            }
            let tx = conn.unchecked_transaction().map_err(e)?;
            tx.execute_batch(SCHEMA_V1).map_err(e)?;
            tx.execute_batch(&migrate_v2()).map_err(e)?;
            tx.execute_batch(&schema_v2()).map_err(e)?;
            tx.pragma_update(None, "user_version", SCHEMA_VERSION)
                .map_err(e)?;
            tx.commit().map_err(e)?;
        }
        SCHEMA_VERSION => {
            require(&TABLES_V1)?;
            require(&TABLES_V2)?;
            conn.execute_batch(SCHEMA_V1).map_err(e)?;
            conn.execute_batch(&schema_v2()).map_err(e)?;
        }
        v => {
            return Err(format!(
                "schema version {v} is newer than this binary supports ({SCHEMA_VERSION})"
            ));
        }
    }
    refresh_usage(conn).map_err(e)?;
    Ok(())
}

/// Optional single row helper (`None` when the query returns no row).
pub fn opt<T>(r: rusqlite::Result<T>) -> Result<Option<T>, AppError> {
    match r {
        Ok(v) => Ok(Some(v)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp(name: &str) -> std::path::PathBuf {
        let p = std::env::temp_dir().join(format!("od-dbtest-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).expect("mkdir");
        p
    }

    #[test]
    fn refuses_damaged_or_foreign_files() {
        let dir = temp("damaged");
        let garbage = dir.join("garbage.db");
        fs::write(&garbage, vec![0x42u8; 8192]).expect("write");
        assert!(Db::open(&garbage).is_err());

        let foreign = dir.join("foreign.db");
        {
            let c = Connection::open(&foreign).expect("open");
            c.execute_batch("CREATE TABLE stuff(x)").expect("create");
        }
        assert!(Db::open(&foreign).is_err());

        let newer = dir.join("newer.db");
        {
            let c = Connection::open(&newer).expect("open");
            c.pragma_update(None, "user_version", 99).expect("pragma");
        }
        assert!(Db::open(&newer).is_err());

        let fresh = dir.join("fresh.db");
        assert!(Db::open(&fresh).is_ok());
        // reopening a valid database works
        assert!(Db::open(&fresh).is_ok());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn migrates_v1_and_keeps_running_totals() {
        let dir = temp("migrate");
        let path = dir.join("v1.db");
        {
            let c = Connection::open(&path).expect("open");
            c.execute_batch(SCHEMA_V1).expect("v1");
            c.pragma_update(None, "user_version", 1).expect("pragma");
            c.execute(
                "INSERT INTO parcels(code, kind, state, total_bytes, expiry_hours, created_at, touched_at) \
                 VALUES ('123456', 0, 1, 100, 1, 0, 0)",
                [],
            )
            .expect("parcel");
            c.execute(
                "INSERT INTO files(parcel_id, idx, name, size, done) VALUES (1, 0, 'a', 60, 1), (1, 1, 'b', 40, 1)",
                [],
            )
            .expect("files");
        }
        let db = Db::open(&path).expect("migrate");
        let u = db.blocking(|c| usage(c)).expect("usage");
        assert_eq!(u.used, 100 + 3 * ENTRY_OVERHEAD);
        assert_eq!((u.parcels, u.pending_bytes, u.pending_files), (1, 0, 0));

        // an unfinished file is pending until it is done or deleted
        db.blocking(|c| {
            c.execute(
                "INSERT INTO parcels(code, kind, state, total_bytes, expiry_hours, created_at, touched_at, charged) \
                 VALUES ('654321', 0, 0, 7, 1, 0, 0, 7)",
                [],
            )?;
            let id = c.last_insert_rowid();
            c.execute(
                "INSERT INTO files(parcel_id, idx, name, size, done) VALUES (?1, 0, 'x', 7, 0), (?1, 1, 'y', 5, 0)",
                [id],
            )?;
            c.execute("UPDATE files SET done = 1 WHERE parcel_id = ?1 AND idx = 1", [id])?;
            Ok(())
        })
        .expect("insert");
        let u = db.blocking(|c| usage(c)).expect("usage");
        assert_eq!(u.used, 100 + 3 * ENTRY_OVERHEAD + 7);
        assert_eq!((u.parcels, u.pending_bytes, u.pending_files), (2, 7, 1));

        // deleting cascades to files; only the ready parcel's code retires
        db.blocking(|c| {
            c.execute("DELETE FROM parcels", [])?;
            Ok(())
        })
        .expect("delete");
        let u = db.blocking(|c| usage(c)).expect("usage");
        assert_eq!(u, Usage::default());
        let retired: Vec<String> = db
            .blocking(|c| {
                let mut s = c.prepare("SELECT code FROM retired_codes")?;
                let rows = s.query_map([], |r| r.get(0))?;
                let mut out = Vec::new();
                for r in rows {
                    out.push(r?);
                }
                Ok(out)
            })
            .expect("retired");
        assert_eq!(retired, ["123456"]);
        drop(db);
        // the migrated file opens as version 2
        assert!(Db::open(&path).is_ok());
        let _ = fs::remove_dir_all(&dir);
    }
}
