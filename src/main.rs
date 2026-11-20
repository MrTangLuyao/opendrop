//! Command line entry point: serve, health, admin-reset, status, version.

#![forbid(unsafe_code)]

use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, TcpStream};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use opendrop::auth::AdminInit;
use opendrop::config::{Config, DEFAULT_ADDR, parse_addr};
use opendrop::db::Db;
use opendrop::settings::Settings;
use opendrop::{App, VERSION, auth, disk};

const USAGE: &str = "usage: opendrop [command]

commands:
  serve                               run the server (default)
  health                              check the local server, exit 0 when healthy
  admin-reset [--password P] [--json] set a new admin password and sign admins out
  status [--json]                     disk, quota and usage from the database
  version                             print the version

environment: OPENDROP_DATA, OPENDROP_ADDR, OPENDROP_TRUST_PROXY,
             OPENDROP_ADMIN_PASSWORD, OPENDROP_SECURE_COOKIE
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let rest = args.get(1..).unwrap_or(&[]);
    match args.first().map(String::as_str) {
        None | Some("serve") if rest.is_empty() => serve(),
        Some("health") if rest.is_empty() => health(),
        Some("admin-reset") => admin_reset(rest),
        Some("status") => status(rest),
        Some("version" | "--version" | "-V") => {
            opendrop::log!("opendrop {VERSION}");
            ExitCode::SUCCESS
        }
        Some("help" | "--help" | "-h") => {
            opendrop::log!("{}", USAGE.trim_end());
            ExitCode::SUCCESS
        }
        _ => usage_error(),
    }
}

fn usage_error() -> ExitCode {
    let _ = std::io::stderr().write_all(USAGE.as_bytes());
    ExitCode::from(2)
}

fn fail(msg: impl std::fmt::Display) -> ExitCode {
    let _ = writeln!(std::io::stderr(), "opendrop: {msg}");
    ExitCode::from(1)
}

fn serve() -> ExitCode {
    let cfg = match Config::from_env() {
        Ok(c) => c,
        Err(e) => return fail(e),
    };
    let addr = cfg.addr;
    let (app, admin) = match App::open(cfg) {
        Ok(v) => v,
        Err(e) => return fail(e),
    };
    match admin {
        AdminInit::Generated(pw) => opendrop::log!("admin password: {pw}  (change it at /admin)"),
        AdminInit::FromEnv => opendrop::log!("admin account created with OPENDROP_ADMIN_PASSWORD"),
        AdminInit::Existing => {}
    }
    // Blocking threads carry file IO and SQLite; 64 is plenty and keeps the
    // process well inside the container's pids limit (256).
    let rt = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .max_blocking_threads(64)
        .build()
    {
        Ok(rt) => rt,
        Err(e) => return fail(format!("cannot start runtime: {e}")),
    };
    let code = rt.block_on(async move {
        let listener = match tokio::net::TcpListener::bind(addr).await {
            Ok(l) => l,
            Err(e) => return fail(format!("cannot listen on {addr}: {e}")),
        };
        let data = app.cfg.data_dir.display().to_string();
        opendrop::log!("opendrop {VERSION} listening on {addr}, data in {data}");
        opendrop::run(Arc::clone(&app), listener, shutdown_signal()).await;
        ExitCode::SUCCESS
    });
    rt.shutdown_timeout(Duration::from_secs(5));
    code
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        match signal(SignalKind::terminate()) {
            Ok(mut term) => {
                tokio::select! {
                    _ = term.recv() => {}
                    _ = tokio::signal::ctrl_c() => {}
                }
            }
            Err(e) => {
                opendrop::log!("warning: cannot watch SIGTERM: {e}");
                let _ = tokio::signal::ctrl_c().await;
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

/// GET /healthz on the configured port with a 1 s budget per step.
fn health() -> ExitCode {
    let raw = std::env::var("OPENDROP_ADDR").unwrap_or_else(|_| DEFAULT_ADDR.to_owned());
    let addr = match parse_addr(if raw.trim().is_empty() {
        DEFAULT_ADDR
    } else {
        &raw
    }) {
        Ok(a) => a,
        Err(e) => return fail(e),
    };
    let ip = match addr.ip() {
        IpAddr::V4(v) if v.is_unspecified() => IpAddr::V4(Ipv4Addr::LOCALHOST),
        IpAddr::V6(v) if v.is_unspecified() => IpAddr::V6(Ipv6Addr::LOCALHOST),
        ip => ip,
    };
    let target = SocketAddr::new(ip, addr.port());
    let timeout = Duration::from_secs(1);
    let check = || -> std::io::Result<bool> {
        let mut s = TcpStream::connect_timeout(&target, timeout)?;
        s.set_read_timeout(Some(timeout))?;
        s.set_write_timeout(Some(timeout))?;
        s.write_all(b"GET /healthz HTTP/1.0\r\nHost: localhost\r\n\r\n")?;
        let mut buf = [0u8; 64];
        let mut n = 0;
        while n < 12 {
            let r = s.read(&mut buf[n..])?;
            if r == 0 {
                break;
            }
            n += r;
        }
        let head = buf.get(..n).unwrap_or(&[]);
        Ok(head.starts_with(b"HTTP/1.1 200") || head.starts_with(b"HTTP/1.0 200"))
    };
    match check() {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => fail("unhealthy"),
        Err(e) => fail(format!("health check failed: {e}")),
    }
}

fn open_db() -> Result<(Config, Db), String> {
    let cfg = Config::from_env()?;
    let path = cfg.data_dir.join("opendrop.db");
    if !path.exists() {
        return Err(format!("no database at {}", path.display()));
    }
    let db = Db::open(&path)?;
    Ok((cfg, db))
}

fn admin_reset(args: &[String]) -> ExitCode {
    let mut password = None;
    let mut json = false;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--json" => json = true,
            "--password" => match it.next() {
                Some(p) => password = Some(p.clone()),
                None => return usage_error(),
            },
            _ => {
                if let Some(p) = a.strip_prefix("--password=") {
                    password = Some(p.to_owned());
                } else {
                    return usage_error();
                }
            }
        }
    }
    let cfg = match Config::from_env() {
        Ok(c) => c,
        Err(e) => return fail(e),
    };
    #[cfg(unix)]
    {
        rustix::process::umask(rustix::fs::Mode::from_raw_mode(0o077));
    }
    if let Err(e) = std::fs::create_dir_all(&cfg.data_dir) {
        return fail(format!("cannot create {}: {e}", cfg.data_dir.display()));
    }
    let db = match Db::open(&cfg.data_dir.join("opendrop.db")) {
        Ok(db) => db,
        Err(e) => return fail(e),
    };
    match auth::reset_admin(&db, password) {
        Ok((username, password)) => {
            if json {
                opendrop::log!(
                    "{}",
                    serde_json::json!({"username": username, "password": password})
                );
            } else {
                opendrop::log!("admin username: {username}");
                opendrop::log!("admin password: {password}");
            }
            ExitCode::SUCCESS
        }
        Err(e) => fail(e),
    }
}

fn status(args: &[String]) -> ExitCode {
    let json = match args {
        [] => false,
        [a] if a == "--json" => true,
        _ => return usage_error(),
    };
    let (cfg, db) = match open_db() {
        Ok(v) => v,
        Err(e) => return fail(e),
    };
    let d = match disk::disk_info(&cfg.data_dir) {
        Ok(d) => d,
        Err(e) => return fail(format!("cannot stat {}: {e}", cfg.data_dir.display())),
    };
    let counts = db.blocking(|c| {
        let s = Settings::load(c, d.total)?;
        let row: (i64, i64, i64, i64) = c.query_row(
            "SELECT (SELECT used FROM usage WHERE id = 1), \
             (SELECT COUNT(*) FROM parcels WHERE state = 1), \
             (SELECT COUNT(*) FROM parcels WHERE state = 0), \
             (SELECT COUNT(*) FROM accounts WHERE is_admin = 0)",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )?;
        Ok((s, row))
    });
    let (s, (used, ready, uploading, accounts)) = match counts {
        Ok(v) => v,
        Err(e) => return fail(format!("database: {}", e.code)),
    };
    let quota = disk::quota(&s, d.total);
    let reserve = disk::reserve(d.total);
    if json {
        let v = serde_json::json!({
            "version": VERSION,
            "data": cfg.data_dir.display().to_string(),
            "disk": {"total": d.total, "avail": d.avail},
            "used": used,
            "quota": quota,
            "reserve": reserve,
            "parcels": ready,
            "uploading": uploading,
            "accounts": accounts,
            "settings": s.to_json(),
        });
        opendrop::log!("{v}");
    } else {
        let gib = |b: u64| b as f64 / (1u64 << 30) as f64;
        opendrop::log!("opendrop {VERSION}");
        opendrop::log!("data      {}", cfg.data_dir.display());
        opendrop::log!(
            "disk      {:.1} GiB total, {:.1} GiB free",
            gib(d.total),
            gib(d.avail)
        );
        opendrop::log!(
            "used      {:.2} GiB of {:.1} GiB quota",
            gib(u64::try_from(used).unwrap_or(0)),
            gib(quota)
        );
        opendrop::log!("parcels   {ready} ready, {uploading} uploading");
        opendrop::log!("accounts  {accounts}");
    }
    ExitCode::SUCCESS
}
