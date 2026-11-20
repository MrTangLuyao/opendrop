//! Runtime configuration from environment variables (all optional).

use std::env;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

/// A write that makes no progress for this long fails the connection.
pub const WRITE_STALL: Duration = Duration::from_secs(60);
/// A pending read fails the connection when nothing moved either way for
/// this long (a backstop; uploads answer 408 after `UPLOAD_IDLE`).
pub const READ_IDLE: Duration = Duration::from_secs(75);
/// An upload that receives nothing for this long is aborted (408).
pub const UPLOAD_IDLE: Duration = Duration::from_secs(60);
/// Every window of this length an upload must bring `UPLOAD_MIN_BYTES`
/// (about 1 KiB/s), so a trickle of a byte a minute cannot hold it open.
pub const UPLOAD_WINDOW: Duration = Duration::from_secs(60);
pub const UPLOAD_MIN_BYTES: u64 = 64 * 1024;

/// Progress deadlines of connections and uploads. Production uses the
/// constants above; tests shorten them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pace {
    pub write_stall: Duration,
    pub read_idle: Duration,
    pub upload_idle: Duration,
    pub upload_window: Duration,
    pub upload_min_bytes: u64,
}

impl Default for Pace {
    fn default() -> Self {
        Pace {
            write_stall: WRITE_STALL,
            read_idle: READ_IDLE,
            upload_idle: UPLOAD_IDLE,
            upload_window: UPLOAD_WINDOW,
            upload_min_bytes: UPLOAD_MIN_BYTES,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SecureCookie {
    /// `Secure` when the request came in over https.
    Auto,
    Always,
    Never,
}

#[derive(Clone, Debug)]
pub struct Config {
    pub data_dir: PathBuf,
    pub addr: SocketAddr,
    /// Take client IP and scheme from X-Forwarded-For / X-Forwarded-Proto.
    pub trust_proxy: bool,
    /// Password for the admin account, used only when it is first created.
    pub admin_password: Option<String>,
    pub secure_cookie: SecureCookie,
    /// Run the periodic sweeper task (tests turn it off and sweep by hand).
    pub background_sweep: bool,
    pub pace: Pace,
}

pub const DEFAULT_ADDR: &str = "0.0.0.0:8080";

impl Config {
    /// Defaults for a given data directory.
    pub fn new(data_dir: PathBuf) -> Self {
        Config {
            data_dir,
            addr: SocketAddr::from(([0, 0, 0, 0], 8080)),
            trust_proxy: false,
            admin_password: None,
            secure_cookie: SecureCookie::Auto,
            background_sweep: true,
            pace: Pace::default(),
        }
    }

    pub fn from_env() -> Result<Config, String> {
        let data_dir = match env::var_os("OPENDROP_DATA") {
            Some(v) if !v.is_empty() => PathBuf::from(v),
            _ => PathBuf::from("./data"),
        };
        let mut cfg = Config::new(data_dir);
        cfg.addr = parse_addr(&var("OPENDROP_ADDR")?.unwrap_or_else(|| DEFAULT_ADDR.to_owned()))?;
        if let Some(v) = var("OPENDROP_TRUST_PROXY")? {
            cfg.trust_proxy = parse_bool("OPENDROP_TRUST_PROXY", &v)?;
        }
        cfg.admin_password = var("OPENDROP_ADMIN_PASSWORD")?;
        if let Some(v) = var("OPENDROP_SECURE_COOKIE")? {
            cfg.secure_cookie = match v.to_ascii_lowercase().as_str() {
                "auto" => SecureCookie::Auto,
                "true" | "1" | "yes" | "on" => SecureCookie::Always,
                "false" | "0" | "no" | "off" => SecureCookie::Never,
                _ => {
                    return Err(format!(
                        "OPENDROP_SECURE_COOKIE must be auto, true or false (got {v:?})"
                    ));
                }
            };
        }
        Ok(cfg)
    }
}

/// Reads a variable; empty means unset. Non UTF-8 values are an error.
fn var(name: &str) -> Result<Option<String>, String> {
    match env::var(name) {
        Ok(v) if v.trim().is_empty() => Ok(None),
        Ok(v) => Ok(Some(v)),
        Err(env::VarError::NotPresent) => Ok(None),
        Err(env::VarError::NotUnicode(_)) => Err(format!("{name} is not valid UTF-8")),
    }
}

pub fn parse_addr(v: &str) -> Result<SocketAddr, String> {
    let v = v.trim();
    if let Some(port) = v.strip_prefix(':') {
        let port: u16 = port
            .parse()
            .map_err(|_| format!("OPENDROP_ADDR: bad port in {v:?}"))?;
        return Ok(SocketAddr::from(([0, 0, 0, 0], port)));
    }
    v.parse()
        .map_err(|_| format!("OPENDROP_ADDR must look like 0.0.0.0:8080 or [::]:8080 (got {v:?})"))
}

fn parse_bool(name: &str, v: &str) -> Result<bool, String> {
    match v.trim().to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" | "on" => Ok(true),
        "false" | "0" | "no" | "off" => Ok(false),
        _ => Err(format!("{name} must be true or false (got {v:?})")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addr_parsing() {
        assert_eq!(parse_addr("127.0.0.1:9000").map(|a| a.port()), Ok(9000));
        assert_eq!(parse_addr(":81").map(|a| a.port()), Ok(81));
        assert!(parse_addr("[::]:8080").is_ok());
        assert!(parse_addr("localhost").is_err());
        assert_eq!(parse_bool("X", "TRUE"), Ok(true));
        assert!(parse_bool("X", "maybe").is_err());
    }
}
