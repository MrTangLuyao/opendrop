//! Admin editable settings: typed, validated, stored as JSON values in the
//! `settings` table and cached in memory.

use rusqlite::Connection;
use serde_json::{Map, Value, json};

use crate::util::{AppError, GIB, MIB};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StorageMode {
    Percent,
    Bytes,
}

impl StorageMode {
    pub fn as_str(self) -> &'static str {
        match self {
            StorageMode::Percent => "percent",
            StorageMode::Bytes => "bytes",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    pub storage_mode: StorageMode,
    pub storage_percent: u32,
    pub storage_bytes: u64,
    pub max_upload_bytes: u64,
    pub max_expiry_hours: u32,
    pub anon_upload: bool,
    pub registration: bool,
    /// Shown as the wordmark and in the footer; split at the first dot.
    pub site_name: String,
}

pub const MIN_STORAGE_BYTES: u64 = 64 * MIB;
pub const MIN_UPLOAD_BYTES: u64 = MIB;
pub const MAX_EXPIRY_HOURS: u32 = 8760;

pub const MAX_SITE_NAME_CHARS: usize = 32;

pub const KEYS: [&str; 8] = [
    "storage_mode",
    "storage_percent",
    "storage_bytes",
    "max_upload_bytes",
    "max_expiry_hours",
    "anon_upload",
    "registration",
    "site_name",
];

impl Settings {
    /// Defaults; byte values are capped to the disk size when it is known so
    /// that saving the untouched settings form never fails validation.
    pub fn defaults(disk_total: u64) -> Self {
        let cap = |v: u64, min: u64| {
            if disk_total > 0 {
                v.min(disk_total).max(min)
            } else {
                v
            }
        };
        Settings {
            storage_mode: StorageMode::Percent,
            storage_percent: 70,
            storage_bytes: cap(30 * GIB, MIN_STORAGE_BYTES),
            max_upload_bytes: cap(5 * GIB, MIN_UPLOAD_BYTES),
            max_expiry_hours: 168,
            anon_upload: true,
            registration: true,
            site_name: "open.drop".to_owned(),
        }
    }

    pub fn to_json(&self) -> Value {
        json!({
            "storage_mode": self.storage_mode.as_str(),
            "storage_percent": self.storage_percent,
            "storage_bytes": self.storage_bytes,
            "max_upload_bytes": self.max_upload_bytes,
            "max_expiry_hours": self.max_expiry_hours,
            "anon_upload": self.anon_upload,
            "registration": self.registration,
            "site_name": self.site_name,
        })
    }

    /// Applies one field. `disk_total` bounds the byte limits (admin input);
    /// `None` skips that bound (values loaded from the database).
    fn set(&mut self, key: &str, v: &Value, disk_total: Option<u64>) -> Result<(), AppError> {
        let bad = || AppError::bad_field(key);
        let within_disk = |n: u64| disk_total.is_none_or(|t| n <= t);
        match key {
            "storage_mode" => {
                self.storage_mode = match v.as_str() {
                    Some("percent") => StorageMode::Percent,
                    Some("bytes") => StorageMode::Bytes,
                    _ => return Err(bad()),
                }
            }
            "storage_percent" => {
                let n = v
                    .as_u64()
                    .filter(|n| (1..=95).contains(n))
                    .ok_or_else(bad)?;
                self.storage_percent = u32::try_from(n).map_err(|_| bad())?;
            }
            "storage_bytes" => {
                let n = v
                    .as_u64()
                    .filter(|&n| n >= MIN_STORAGE_BYTES && within_disk(n) && n <= i64::MAX as u64)
                    .ok_or_else(bad)?;
                self.storage_bytes = n;
            }
            "max_upload_bytes" => {
                let n = v
                    .as_u64()
                    .filter(|&n| n >= MIN_UPLOAD_BYTES && within_disk(n) && n <= i64::MAX as u64)
                    .ok_or_else(bad)?;
                self.max_upload_bytes = n;
            }
            "max_expiry_hours" => {
                let n = v
                    .as_u64()
                    .filter(|n| (1..=u64::from(MAX_EXPIRY_HOURS)).contains(n))
                    .ok_or_else(bad)?;
                self.max_expiry_hours = u32::try_from(n).map_err(|_| bad())?;
            }
            "anon_upload" => self.anon_upload = v.as_bool().ok_or_else(bad)?,
            "registration" => self.registration = v.as_bool().ok_or_else(bad)?,
            "site_name" => {
                let name = v.as_str().map(str::trim).ok_or_else(bad)?;
                let n = name.chars().count();
                // Control and direction-override characters could disguise the name.
                let hidden = |c: char| {
                    c.is_control()
                        || matches!(c, '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{FEFF}')
                };
                if n == 0 || n > MAX_SITE_NAME_CHARS || name.chars().any(hidden) {
                    return Err(bad());
                }
                self.site_name = name.to_owned();
            }
            _ => return Err(bad()),
        }
        Ok(())
    }

    /// Validates every field of an admin update; unknown keys are rejected.
    pub fn apply(&self, body: &Map<String, Value>, disk_total: u64) -> Result<Settings, AppError> {
        let mut next = self.clone();
        for (k, v) in body {
            next.set(k, v, Some(disk_total))?;
        }
        Ok(next)
    }

    pub fn load(conn: &Connection, disk_total: u64) -> Result<Settings, AppError> {
        let mut s = Settings::defaults(disk_total);
        let mut stmt = conn.prepare("SELECT key, value FROM settings")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        for row in rows {
            let (k, raw) = row?;
            let parsed: Option<Value> = serde_json::from_str(&raw).ok();
            let ok = parsed.is_some_and(|v| s.set(&k, &v, None).is_ok());
            if !ok {
                crate::log!("warning: ignoring invalid stored setting {k:?}");
            }
        }
        Ok(s)
    }

    pub fn save(&self, conn: &Connection) -> Result<(), AppError> {
        let tx = conn.unchecked_transaction()?;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO settings(key, value) VALUES (?1, ?2) \
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            )?;
            let all = self.to_json();
            for key in KEYS {
                let v = all.get(key).cloned().unwrap_or(Value::Null);
                stmt.execute((key, v.to_string()))?;
            }
        }
        tx.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obj(v: Value) -> Map<String, Value> {
        match v {
            Value::Object(m) => m,
            _ => Map::new(),
        }
    }

    #[test]
    fn validation() {
        let total = 100 * GIB;
        let s = Settings::defaults(total);
        let ok = s
            .apply(
                &obj(
                    json!({"storage_mode":"bytes","storage_bytes": 10 * GIB,"max_upload_bytes": MIB,
                            "max_expiry_hours": 8760,"anon_upload": false,"registration": false,
                            "storage_percent": 95}),
                ),
                total,
            )
            .expect("valid");
        assert_eq!(ok.storage_mode, StorageMode::Bytes);
        assert_eq!(ok.storage_bytes, 10 * GIB);
        assert!(!ok.anon_upload && !ok.registration);
        assert_eq!(ok.storage_percent, 95);
        let named = s
            .apply(&obj(json!({"site_name": "  drop.louie1.com  "})), total)
            .expect("name");
        assert_eq!(named.site_name, "drop.louie1.com");
        assert_eq!(s.site_name, "open.drop");

        let cases = [
            (json!({"storage_mode":"half"}), "storage_mode"),
            (json!({"storage_percent":0}), "storage_percent"),
            (json!({"storage_percent":96}), "storage_percent"),
            (json!({"storage_percent":30.5}), "storage_percent"),
            (json!({"storage_percent":"30"}), "storage_percent"),
            (json!({"storage_bytes": 64 * MIB - 1}), "storage_bytes"),
            (json!({"storage_bytes": total + 1}), "storage_bytes"),
            (json!({"storage_bytes": -5}), "storage_bytes"),
            (json!({"max_upload_bytes": MIB - 1}), "max_upload_bytes"),
            (json!({"max_upload_bytes": total + 1}), "max_upload_bytes"),
            (json!({"max_expiry_hours": 0}), "max_expiry_hours"),
            (json!({"max_expiry_hours": 8761}), "max_expiry_hours"),
            (json!({"anon_upload": "yes"}), "anon_upload"),
            (json!({"registration": 1}), "registration"),
            (json!({"bogus": 1}), "bogus"),
            (json!({"max_expiry_hours": null}), "max_expiry_hours"),
            (json!({"site_name": ""}), "site_name"),
            (json!({"site_name": "   "}), "site_name"),
            (json!({"site_name": "a".repeat(33)}), "site_name"),
            (json!({"site_name": "evil\u{202E}gnp"}), "site_name"),
            (json!({"site_name": "line\nbreak"}), "site_name"),
            (json!({"site_name": 5}), "site_name"),
        ];
        for (body, field) in cases {
            let err = s.apply(&obj(body.clone()), total).expect_err("must fail");
            assert_eq!(err.code, "bad_field", "{body}");
            assert_eq!(err.extra.get("field"), Some(&Value::from(field)), "{body}");
        }
        // the defaults always validate against the disk they were made for
        let small = 20 * GIB;
        let d = Settings::defaults(small);
        let all = obj(d.to_json());
        assert_eq!(d.apply(&all, small).expect("defaults valid"), d);
    }

    #[test]
    fn roundtrip_through_db() {
        let conn = Connection::open_in_memory().expect("db");
        conn.execute_batch("CREATE TABLE settings(key TEXT PRIMARY KEY, value TEXT NOT NULL)")
            .expect("schema");
        let mut s = Settings::defaults(GIB * 100);
        s.storage_mode = StorageMode::Bytes;
        s.max_expiry_hours = 24;
        s.registration = false;
        s.save(&conn).expect("save");
        let loaded = Settings::load(&conn, GIB * 100).expect("load");
        assert_eq!(loaded, s);
        conn.execute(
            "UPDATE settings SET value='\"x\"' WHERE key='max_expiry_hours'",
            [],
        )
        .expect("corrupt");
        let loaded = Settings::load(&conn, GIB * 100).expect("load");
        assert_eq!(loaded.max_expiry_hours, 168);
    }
}
