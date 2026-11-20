//! opendrop: self-hosted temporary file drop with six digit pickup codes.

#![forbid(unsafe_code)]

pub mod admin;
pub mod assets;
pub mod auth;
pub mod body;
pub mod config;
pub mod conn;
pub mod db;
pub mod disk;
pub mod download;
pub mod http;
pub mod parcels;
pub mod qr;
pub mod ratelimit;
pub mod settings;
pub mod sweep;
pub mod util;
pub mod zip;

use std::future::Future;
use std::path::PathBuf;
use std::sync::{Arc, PoisonError, RwLock};

use tokio::net::TcpListener;
use tokio::sync::Semaphore;

use crate::auth::{AdminInit, HASH_THREADS, HashPool};
use crate::config::Config;
use crate::db::Db;
use crate::download::Streams;
use crate::parcels::InFlight;
use crate::ratelimit::{ClientSlots, RateLimits};
use crate::settings::Settings;
use crate::zip::PlanCache;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// QR codes rendered at once (on the blocking pool).
pub const QR_RENDERS: usize = 2;
/// Parcel creation bodies above 64 KiB buffered at once, server wide.
pub const BIG_BODIES: usize = 32;
/// Downloads streaming at once, server wide. A peer that stops reading
/// pins one 256 KiB chunk until the write stall drops it, so this bounds
/// that memory (about 150 MiB) however many addresses an attacker has.
pub const DOWNLOADS: usize = 512;

/// Shared server state.
pub struct App {
    pub cfg: Config,
    pub db: Db,
    pub files_dir: PathBuf,
    settings: RwLock<Settings>,
    pub limits: RateLimits,
    pub slots: ClientSlots,
    pub inflight: InFlight,
    /// Parcels with downloads streaming (their files stay until done).
    pub streams: Arc<Streams>,
    /// Zip layouts shared by concurrent downloads of a parcel.
    pub zips: PlanCache,
    /// Bounds concurrent argon2 work (19 MiB each).
    pub hash_sem: Arc<Semaphore>,
    pub hasher: HashPool,
    pub qr_sem: Arc<Semaphore>,
    pub body_sem: Arc<Semaphore>,
    pub download_sem: Arc<Semaphore>,
    /// A real hash, verified against when the account does not exist.
    pub dummy_hash: String,
    /// Keeps sweeps single flight.
    pub sweep_lock: tokio::sync::Mutex<()>,
}

impl App {
    /// Prepares the data directory and database. Synchronous: call before
    /// serving (it may hash a password).
    pub fn open(cfg: Config) -> Result<(Arc<App>, AdminInit), String> {
        #[cfg(unix)]
        {
            // data files and directories are private to the service user
            rustix::process::umask(rustix::fs::Mode::from_raw_mode(0o077));
        }
        let data = &cfg.data_dir;
        std::fs::create_dir_all(data)
            .map_err(|e| format!("cannot create data directory {}: {e}", data.display()))?;
        let files_dir = data.join("files");
        std::fs::create_dir_all(&files_dir)
            .map_err(|e| format!("cannot create {}: {e}", files_dir.display()))?;
        let db = Db::open(&data.join("opendrop.db"))?;
        // New parcel ids must never land in a directory that is already
        // there (a database restored from a backup, or a new database next
        // to old files), or old blobs would mix with new ones.
        let highest = sweep::highest_dir_id(&files_dir)
            .map_err(|e| format!("cannot read {}: {e}", files_dir.display()))?;
        db.blocking(|c| db::reserve_ids(c, highest))
            .map_err(|e| format!("database: {}", e.code))?;
        let disk =
            disk::disk_info(data).map_err(|e| format!("cannot stat {}: {e}", data.display()))?;
        let settings = db
            .blocking(|c| Settings::load(c, disk.total))
            .map_err(|e| format!("cannot load settings: {}", e.code))?;
        let admin = auth::ensure_admin(&db, cfg.admin_password.as_deref())?;
        let dummy_hash = auth::hash_blocking("opendrop timing equaliser")
            .map_err(|e| format!("argon2: {}", e.code))?;
        let app = App {
            cfg,
            db,
            files_dir,
            settings: RwLock::new(settings),
            limits: RateLimits::new(),
            slots: ClientSlots::new(),
            inflight: InFlight::default(),
            streams: Arc::new(Streams::default()),
            zips: PlanCache::default(),
            hash_sem: Arc::new(Semaphore::new(HASH_THREADS)),
            hasher: HashPool::new()?,
            qr_sem: Arc::new(Semaphore::new(QR_RENDERS)),
            body_sem: Arc::new(Semaphore::new(BIG_BODIES)),
            download_sem: Arc::new(Semaphore::new(DOWNLOADS)),
            dummy_hash,
            sweep_lock: tokio::sync::Mutex::new(()),
        };
        Ok((Arc::new(app), admin))
    }

    pub fn settings(&self) -> Settings {
        self.settings
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub fn set_settings(&self, s: Settings) {
        *self
            .settings
            .write()
            .unwrap_or_else(PoisonError::into_inner) = s;
    }

    pub fn parcel_dir(&self, id: i64) -> PathBuf {
        self.files_dir.join(id.to_string())
    }
}

/// Runs the startup sweep, the background sweeper and the HTTP server until
/// `shutdown` resolves.
pub async fn run(app: Arc<App>, listener: TcpListener, shutdown: impl Future<Output = ()>) {
    if let Err(e) = sweep::run(&app, true).await {
        crate::log!("warning: startup sweep failed: {}", e.code);
    }
    let sweeper = app
        .cfg
        .background_sweep
        .then(|| sweep::spawn(Arc::clone(&app)));
    http::serve(Arc::clone(&app), listener, shutdown).await;
    if let Some(task) = sweeper {
        task.abort();
    }
}
