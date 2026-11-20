//! Shared helpers: the error type, time, random tokens, file names, encodings.

use std::collections::{HashMap, HashSet};
use std::fmt::{Display, Write as _};
use std::time::{SystemTime, UNIX_EPOCH};

use http::StatusCode;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

pub const KIB: u64 = 1 << 10;
pub const MIB: u64 = 1 << 20;
pub const GIB: u64 = 1 << 30;

pub const SECOND_MS: i64 = 1000;
pub const HOUR_MS: i64 = 3_600_000;
pub const DAY_MS: i64 = 24 * HOUR_MS;
pub const WEEK_MS: i64 = 7 * DAY_MS;

/// Writes one log line to stdout. Unlike `println!` a closed or broken
/// stdout is ignored instead of panicking.
pub fn log_line(args: std::fmt::Arguments<'_>) {
    use std::io::Write as _;
    let mut out = std::io::stdout().lock();
    let _ = out.write_fmt(args);
    let _ = out.write_all(b"\n");
    let _ = out.flush();
}

/// `log!("...", args)`: one line on stdout (see [`log_line`]).
#[macro_export]
macro_rules! log {
    ($($arg:tt)*) => {
        $crate::util::log_line(format_args!($($arg)*))
    };
}

/// Current unix time in milliseconds.
pub fn now_ms() -> i64 {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(d) => i64::try_from(d.as_millis()).unwrap_or(i64::MAX),
        Err(_) => 0,
    }
}

/// `now + hours`, saturating instead of overflowing.
pub fn after_hours(now: i64, hours: i64) -> i64 {
    now.saturating_add(hours.saturating_mul(HOUR_MS))
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Every failure a handler can produce. Rendered as `{"error": code, ...extra}`.
#[derive(Debug)]
pub struct AppError {
    pub status: StatusCode,
    pub code: &'static str,
    pub extra: Map<String, Value>,
    pub retry_after: Option<u64>,
}

impl AppError {
    pub fn new(status: StatusCode, code: &'static str) -> Self {
        AppError {
            status,
            code,
            extra: Map::new(),
            retry_after: None,
        }
    }

    pub fn with(mut self, key: &str, value: impl Into<Value>) -> Self {
        self.extra.insert(key.to_owned(), value.into());
        self
    }

    pub fn bad_field(field: &str) -> Self {
        Self::new(StatusCode::BAD_REQUEST, "bad_field").with("field", field)
    }

    pub fn not_found() -> Self {
        Self::new(StatusCode::NOT_FOUND, "not_found")
    }

    pub fn unauthorized() -> Self {
        Self::new(StatusCode::UNAUTHORIZED, "unauthorized")
    }

    pub fn forbidden() -> Self {
        Self::new(StatusCode::FORBIDDEN, "forbidden")
    }

    pub fn bad_token() -> Self {
        Self::new(StatusCode::FORBIDDEN, "bad_token")
    }

    pub fn rate_limited(secs: u64) -> Self {
        let mut e =
            Self::new(StatusCode::TOO_MANY_REQUESTS, "rate_limited").with("retry_after", secs);
        e.retry_after = Some(secs);
        e
    }

    /// 503: the server is at a capacity limit right now.
    pub fn busy(secs: u64) -> Self {
        let mut e = Self::new(StatusCode::SERVICE_UNAVAILABLE, "busy").with("retry_after", secs);
        e.retry_after = Some(secs);
        e
    }

    /// Logs the cause (never sent to the client) and returns a generic 500.
    pub fn internal(context: &str, err: impl Display) -> Self {
        crate::log!("error: {context}: {err}");
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, "internal")
    }

    /// Maps an I/O error, turning "disk full" into 507 instead of 500.
    pub fn io(context: &str, err: std::io::Error) -> Self {
        if is_disk_full(&err) {
            Self::new(StatusCode::INSUFFICIENT_STORAGE, "disk_full")
        } else {
            Self::internal(context, err)
        }
    }

    pub fn to_json(&self) -> Value {
        let mut m = Map::with_capacity(self.extra.len() + 1);
        m.insert("error".to_owned(), Value::from(self.code));
        for (k, v) in &self.extra {
            m.insert(k.clone(), v.clone());
        }
        Value::Object(m)
    }
}

impl From<rusqlite::Error> for AppError {
    fn from(e: rusqlite::Error) -> Self {
        AppError::internal("database", e)
    }
}

pub fn is_disk_full(err: &std::io::Error) -> bool {
    matches!(
        err.kind(),
        std::io::ErrorKind::StorageFull | std::io::ErrorKind::QuotaExceeded
    )
}

// ---------------------------------------------------------------------------
// Randomness and tokens
// ---------------------------------------------------------------------------

pub fn fill_random(buf: &mut [u8]) -> Result<(), AppError> {
    getrandom::fill(buf).map_err(|e| AppError::internal("random", e))
}

/// base64url without padding.
pub fn b64url(data: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b0 = u32::from(chunk.first().copied().unwrap_or(0));
        let b1 = u32::from(chunk.get(1).copied().unwrap_or(0));
        let b2 = u32::from(chunk.get(2).copied().unwrap_or(0));
        let n = (b0 << 16) | (b1 << 8) | b2;
        for i in 0..=chunk.len() {
            let idx = (n >> (18 - 6 * i)) & 63;
            out.push(char::from(ALPHABET[idx as usize]));
        }
    }
    out
}

/// A fresh secret: the text handed to the client and the hash we store.
pub struct Token {
    pub text: String,
    pub hash: [u8; 32],
}

pub fn new_token() -> Result<Token, AppError> {
    let mut raw = [0u8; 32];
    fill_random(&mut raw)?;
    let text = b64url(&raw);
    let hash = hash_token(&text);
    Ok(Token { text, hash })
}

pub fn hash_token(text: &str) -> [u8; 32] {
    Sha256::digest(text.as_bytes()).into()
}

/// Shape check for tokens produced by [`new_token`] (43 base64url chars).
pub fn is_token(s: &str) -> bool {
    s.len() == 43
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// Uniform six digit pickup code.
pub fn random_code() -> Result<String, AppError> {
    // Largest multiple of 1_000_000 that fits in u32; reject above it.
    const LIMIT: u32 = 4_294_000_000;
    loop {
        let mut b = [0u8; 4];
        fill_random(&mut b)?;
        let x = u32::from_le_bytes(b);
        if x < LIMIT {
            return Ok(format!("{:06}", x % 1_000_000));
        }
    }
}

pub fn is_code(s: &str) -> bool {
    s.len() == 6 && s.bytes().all(|b| b.is_ascii_digit())
}

/// Random password of `len` characters from [A-Za-z0-9].
pub fn random_password(len: usize) -> Result<String, AppError> {
    const ALPHABET: &[u8; 62] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    let mut out = String::with_capacity(len);
    let mut buf = [0u8; 64];
    while out.len() < len {
        fill_random(&mut buf)?;
        for &b in &buf {
            // 248 = 4 * 62: rejecting above keeps the choice uniform.
            if b < 248 && out.len() < len {
                out.push(char::from(ALPHABET[usize::from(b % 62)]));
            }
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// File names
// ---------------------------------------------------------------------------

pub const MAX_SEGMENT_BYTES: usize = 255;
pub const MAX_SEGMENTS: usize = 32;
pub const MAX_PATH_BYTES: usize = 1024;

/// Control, bidi and other invisible formatting characters that can be used
/// to disguise a name (for example U+202E turning `exe.txt` around).
pub fn is_hidden_char(c: char) -> bool {
    c.is_control()
        || matches!(
            c,
            '\u{00AD}'
                | '\u{061C}'
                | '\u{180E}'
                | '\u{200B}'..='\u{200F}'
                | '\u{2028}'..='\u{202E}'
                | '\u{2060}'..='\u{206F}'
                | '\u{FEFF}'
                | '\u{FFF9}'..='\u{FFFB}'
        )
}

/// Longest prefix of `s` that is at most `max` bytes and ends on a char boundary.
pub fn truncate_bytes(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    s.get(..end).unwrap_or("")
}

fn trim_segment(s: &str) -> &str {
    s.trim_start_matches(' ').trim_end_matches([' ', '.'])
}

/// Characters Windows refuses in file names. A colon also makes `C:` a drive
/// and `name:stream` an NTFS stream when an archive is extracted there.
fn is_windows_reserved(c: char) -> bool {
    matches!(c, ':' | '<' | '>' | '"' | '|' | '?' | '*')
}

fn clean_segment(seg: &str) -> Option<String> {
    let visible: String = seg
        .chars()
        .filter(|&c| !is_hidden_char(c))
        .map(|c| if is_windows_reserved(c) { '_' } else { c })
        .collect();
    let s = trim_segment(trim_segment(&visible));
    let s = trim_segment(truncate_bytes(s, MAX_SEGMENT_BYTES));
    if s.is_empty() {
        None
    } else {
        Some(s.to_owned())
    }
}

fn joined_len(segs: &[String]) -> usize {
    segs.iter().map(String::len).sum::<usize>() + segs.len().saturating_sub(1)
}

/// Turns a client supplied (possibly relative) path into a safe display path.
///
/// Backslashes become slashes; empty, `.` and `..` segments are dropped;
/// invisible characters and surrounding spaces / trailing dots are removed;
/// characters Windows rejects (`:<>"|?*`) become `_`, so no entry can name a
/// drive or a stream; each segment is cut to 255 bytes, at most 32 segments and 1024 bytes are
/// kept (the leading directories go first). The result never touches the
/// file system: blobs on disk are named by index only.
pub fn sanitize_name(name: &str) -> String {
    let replaced = name.replace('\\', "/");
    let mut segs: Vec<String> = replaced.split('/').filter_map(clean_segment).collect();
    if segs.len() > MAX_SEGMENTS {
        let drop = segs.len() - MAX_SEGMENTS;
        segs.drain(..drop);
    }
    while segs.len() > 1 && joined_len(&segs) > MAX_PATH_BYTES {
        segs.remove(0);
    }
    if segs.is_empty() {
        return "file".to_owned();
    }
    segs.join("/")
}

/// Last path segment (the name a single download is saved as).
pub fn basename(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

fn split_ext(last: &str) -> (&str, &str) {
    match last.rfind('.') {
        Some(i) if i > 0 => (last.get(..i).unwrap_or(last), last.get(i..).unwrap_or("")),
        _ => (last, ""),
    }
}

fn numbered_name(dir: &str, last: &str, n: usize) -> String {
    let suffix = format!(" ({n})");
    let (mut stem, mut ext) = split_ext(last);
    if ext.len() + suffix.len() > MAX_SEGMENT_BYTES / 2 {
        stem = last;
        ext = "";
    }
    let mut stem = truncate_bytes(stem, MAX_SEGMENT_BYTES - suffix.len() - ext.len());
    let mut dir = dir;
    loop {
        let sep = usize::from(!dir.is_empty());
        let total = dir.len() + sep + stem.len() + suffix.len() + ext.len();
        if total <= MAX_PATH_BYTES {
            break;
        }
        match dir.split_once('/') {
            Some((_, rest)) => dir = rest,
            None => dir = "",
        }
    }
    if stem.trim().is_empty() {
        stem = "file";
    }
    if dir.is_empty() {
        format!("{stem}{suffix}{ext}")
    } else {
        format!("{dir}/{stem}{suffix}{ext}")
    }
}

fn join_path(dir: &str, last: &str) -> String {
    if dir.is_empty() {
        last.to_owned()
    } else {
        format!("{dir}/{last}")
    }
}

/// The folders a path lies in: "a" and "a/b" for "a/b/c".
fn dir_prefixes(path: &str) -> impl Iterator<Item = &str> {
    path.match_indices('/')
        .map(move |(i, _)| path.get(..i).unwrap_or(""))
}

/// `seg (n)` for a folder name, kept within the segment limit.
fn numbered_dir(seg: &str, n: usize) -> String {
    let suffix = format!(" ({n})");
    let stem = truncate_bytes(seg, MAX_SEGMENT_BYTES.saturating_sub(suffix.len()));
    format!("{stem}{suffix}")
}

/// Names already handed out by [`dedup_names`], all lowercased.
#[derive(Default)]
struct Taken {
    files: HashSet<String>,
    dirs: HashSet<String>,
    /// Next number to try per clashing name, so that many copies of one
    /// name cost linear time.
    next: HashMap<String, usize>,
    /// Folders renamed because a file had their path: original (lowercased)
    /// to new (as shown, lowercased).
    moved: HashMap<String, (String, String)>,
}

impl Taken {
    fn free(&self, lower: &str) -> bool {
        !self.files.contains(lower) && !self.dirs.contains(lower)
    }

    fn under_a_file(&self, lower: &str) -> bool {
        dir_prefixes(lower).any(|p| self.files.contains(p))
    }

    /// Where the folder `dir` ends up: unchanged unless a file already has
    /// the path of one of its folders, which then becomes `name (n)` for
    /// this and every later file in it. Returns (shown, lowercased).
    fn place_dir(&mut self, dir: &str) -> (String, String) {
        let mut placed = String::new();
        let mut placed_lower = String::new();
        let mut orig_lower = String::new();
        if dir.is_empty() {
            return (placed, placed_lower);
        }
        for seg in dir.split('/') {
            let seg_lower = seg.to_lowercase();
            if !orig_lower.is_empty() {
                orig_lower.push('/');
            }
            orig_lower.push_str(&seg_lower);
            if let Some((m, ml)) = self.moved.get(&orig_lower) {
                placed = m.clone();
                placed_lower = ml.clone();
                continue;
            }
            if !placed_lower.is_empty() {
                placed_lower.push('/');
            }
            placed_lower.push_str(&seg_lower);
            if !self.files.contains(&placed_lower) {
                placed = join_path(&placed, seg);
                continue;
            }
            // a file has this path: the folder gets a number
            let clash = std::mem::take(&mut placed_lower);
            let mut n = self.next.get(&clash).copied().unwrap_or(2);
            let (shown, lower) = loop {
                let shown = join_path(&placed, &numbered_dir(seg, n));
                n = n.saturating_add(1);
                let lower = shown.to_lowercase();
                if self.free(&lower) {
                    break (shown, lower);
                }
            };
            self.next.insert(clash, n);
            self.moved
                .insert(orig_lower.clone(), (shown.clone(), lower.clone()));
            placed = shown;
            placed_lower = lower;
        }
        (placed, placed_lower)
    }

    /// A unique name for `name`, recorded as taken.
    fn claim(&mut self, name: &str) -> String {
        let (dir, last) = match name.rfind('/') {
            Some(i) => (name.get(..i).unwrap_or(""), name.get(i + 1..).unwrap_or("")),
            None => ("", name),
        };
        let (mut dir, _) = self.place_dir(dir);
        let mut path = join_path(&dir, last);
        // a renamed folder made the path longer: drop leading folders, as
        // sanitize_name does
        while path.len() > MAX_PATH_BYTES && !dir.is_empty() {
            dir = dir
                .split_once('/')
                .map(|(_, r)| r.to_owned())
                .unwrap_or_default();
            path = join_path(&dir, last);
        }
        let mut lower = path.to_lowercase();
        if self.under_a_file(&lower) {
            dir.clear();
            path = last.to_owned();
            lower = path.to_lowercase();
        }
        if !self.free(&lower) {
            let key = lower;
            let mut n = self.next.get(&key).copied().unwrap_or(2);
            loop {
                let candidate = numbered_name(&dir, last, n);
                let cl = candidate.to_lowercase();
                if self.under_a_file(&cl) {
                    // only possible after numbered_name shortened the folders
                    dir.clear();
                    continue;
                }
                n = n.saturating_add(1);
                if self.free(&cl) {
                    path = candidate;
                    lower = cl;
                    break;
                }
            }
            self.next.insert(key, n);
        }
        for p in dir_prefixes(&lower) {
            if !self.dirs.contains(p) {
                self.dirs.insert(p.to_owned());
            }
        }
        self.files.insert(lower);
        path
    }
}

/// Makes names unique (case-insensitively) by inserting ` (2)`, ` (3)` ...
/// before the extension of the later duplicates. A file never has the path
/// of a folder either (a zip with `docs` next to `docs/a.txt` cannot be
/// extracted): whichever comes later is renamed, a folder as `docs (2)` for
/// all the files in it. The work is linear in the number of names.
pub fn dedup_names(names: Vec<String>) -> Vec<String> {
    let mut taken = Taken::default();
    names.iter().map(|n| taken.claim(n)).collect()
}

/// `attachment` disposition with an ASCII fallback and an RFC 5987 UTF-8 name.
pub fn content_disposition(filename: &str) -> String {
    let mut ascii = String::with_capacity(filename.len());
    for c in filename.chars() {
        if c == '"' || c == '\\' || c.is_control() {
            continue;
        }
        ascii.push(if c.is_ascii() { c } else { '_' });
    }
    if ascii.trim().is_empty() {
        ascii = "download".to_owned();
    }
    let mut encoded = String::with_capacity(filename.len() * 3);
    for b in filename.bytes() {
        if b.is_ascii_alphanumeric() || b"!#$&+-.^_`|~".contains(&b) {
            encoded.push(char::from(b));
        } else {
            let _ = write!(encoded, "%{b:02X}");
        }
    }
    format!("attachment; filename=\"{ascii}\"; filename*=UTF-8''{encoded}")
}

// ---------------------------------------------------------------------------
// Query strings
// ---------------------------------------------------------------------------

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// Decodes `application/x-www-form-urlencoded` text. `None` on bad escapes
/// or invalid UTF-8.
pub fn form_decode(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while let Some(&b) = bytes.get(i) {
        match b {
            b'+' => out.push(b' '),
            b'%' => {
                let hi = hex_val(*bytes.get(i + 1)?)?;
                let lo = hex_val(*bytes.get(i + 2)?)?;
                out.push((hi << 4) | lo);
                i += 2;
            }
            _ => out.push(b),
        }
        i += 1;
    }
    String::from_utf8(out).ok()
}

/// First value of `key` in a query string, decoded.
pub fn query_param(query: &str, key: &str) -> Option<String> {
    query.split('&').find_map(|pair| {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        if k == key { form_decode(v) } else { None }
    })
}

// ---------------------------------------------------------------------------
// Calendar
// ---------------------------------------------------------------------------

/// Days since 1970-01-01 to (year, month, day), proleptic Gregorian.
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

/// MS-DOS (time, date) for a unix millisecond timestamp, in UTC, clamped to
/// the representable 1980..=2107 range.
pub fn dos_datetime(ms: i64) -> (u16, u16) {
    let secs = ms.div_euclid(1000);
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    if y < 1980 {
        return (0, (1 << 5) | 1);
    }
    if y > 2107 {
        return ((23 << 11) | (59 << 5) | 29, (127 << 9) | (12 << 5) | 31);
    }
    let time = ((rem / 3600) << 11) | (((rem % 3600) / 60) << 5) | ((rem % 60) / 2);
    let date = ((y - 1980) << 9) | (m << 5) | d;
    (
        u16::try_from(time).unwrap_or(0),
        u16::try_from(date).unwrap_or(0),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_traversal() {
        assert_eq!(sanitize_name("../../etc/passwd"), "etc/passwd");
        assert_eq!(
            sanitize_name("..\\..\\windows\\system32"),
            "windows/system32"
        );
        assert_eq!(sanitize_name("/abs/./x/../y"), "abs/x/y");
        assert_eq!(sanitize_name("a//b"), "a/b");
        assert_eq!(sanitize_name(".."), "file");
        assert_eq!(sanitize_name(""), "file");
        assert_eq!(sanitize_name("   "), "file");
        assert_eq!(sanitize_name("..."), "file");
        assert_eq!(sanitize_name(". ./x"), "x");
    }

    #[test]
    fn sanitize_hidden_and_spaces() {
        assert_eq!(sanitize_name("invoice\u{202E}txt.exe"), "invoicetxt.exe");
        assert_eq!(sanitize_name("a\u{0}b\u{1f}c\u{7f}d"), "abcd");
        assert_eq!(sanitize_name("x\u{200B}y\u{FEFF}"), "xy");
        assert_eq!(sanitize_name("line\nbreak\r.txt"), "linebreak.txt");
        assert_eq!(sanitize_name("  name.txt. . "), "name.txt");
        assert_eq!(sanitize_name(" dir /  file "), "dir/file");
        // dotfiles keep their leading dot
        assert_eq!(sanitize_name("proj/.env"), "proj/.env");
        assert_eq!(sanitize_name("\u{0}../x"), "x");
        assert_eq!(sanitize_name("中文 名字.txt"), "中文 名字.txt");
    }

    #[test]
    fn sanitize_windows_names() {
        // no drive letters, no NTFS streams
        assert_eq!(
            sanitize_name("C:\\Users\\Public\\Desktop\\run.bat"),
            "C_/Users/Public/Desktop/run.bat"
        );
        assert_eq!(sanitize_name("readme.txt:payload"), "readme.txt_payload");
        assert_eq!(sanitize_name("a<b>c\"d|e?f*g"), "a_b_c_d_e_f_g");
        assert_eq!(sanitize_name("::"), "__");
    }

    #[test]
    fn sanitize_long_names() {
        let long = "界".repeat(200); // 600 bytes
        let s = sanitize_name(&long);
        assert!(s.len() <= MAX_SEGMENT_BYTES);
        assert!(s.chars().all(|c| c == '界'));
        assert_eq!(s.len(), 255);

        let deep: Vec<String> = (0..40).map(|i| format!("d{i}")).collect();
        let s = sanitize_name(&deep.join("/"));
        assert_eq!(s.split('/').count(), MAX_SEGMENTS);
        assert!(s.ends_with("d39"));

        let seg = "a".repeat(255);
        let p = [seg.as_str(); 10].join("/");
        let s = sanitize_name(&p);
        assert!(s.len() <= MAX_PATH_BYTES);
        assert_eq!(s.split('/').count(), 4);

        // cut at a char boundary followed by trailing dots/spaces trimming
        let tricky = format!("{}. x", "b".repeat(254));
        let s = sanitize_name(&tricky);
        assert_eq!(s, "b".repeat(254));
    }

    #[test]
    fn dedup_case_insensitive() {
        let out = dedup_names(vec![
            "a.txt".into(),
            "A.TXT".into(),
            "a.txt".into(),
            "dir/b".into(),
            "dir/B".into(),
            ".env".into(),
            ".ENV".into(),
            "x.tar.gz".into(),
            "x.tar.gz".into(),
        ]);
        assert_eq!(
            out,
            vec![
                "a.txt",
                "A (2).TXT",
                "a (3).txt",
                "dir/b",
                "dir/B (2)",
                ".env",
                ".ENV (2)",
                "x.tar.gz",
                "x.tar (2).gz",
            ]
        );
    }

    #[test]
    fn dedup_respects_limits() {
        let name = "z".repeat(255);
        let out = dedup_names(vec![name.clone(), name.clone(), name]);
        assert_eq!(out.len(), 3);
        for n in &out {
            assert!(n.len() <= MAX_SEGMENT_BYTES, "{}", n.len());
        }
        assert!(out[1].ends_with(" (2)"));
        let set: HashSet<_> = out.iter().map(|s| s.to_lowercase()).collect();
        assert_eq!(set.len(), 3);

        let long_path = vec!["q".repeat(255); 4].join("/");
        let long_path = sanitize_name(&long_path);
        let out = dedup_names(vec![long_path.clone(), long_path]);
        assert!(out[1].len() <= MAX_PATH_BYTES);
        assert!(out[1].ends_with(" (2)"));
    }

    #[test]
    fn dedup_many_copies_is_linear() {
        // quadratic probing would take minutes here
        let name = format!("{}/{}.bin", "d".repeat(500), "x".repeat(250));
        let out = dedup_names(vec![name; 20_000]);
        let set: HashSet<_> = out.iter().map(|s| s.to_lowercase()).collect();
        assert_eq!(set.len(), 20_000);
        assert!(out.iter().all(|n| n.len() <= MAX_PATH_BYTES));
        assert!(out[19_999].ends_with(" (20000).bin"), "{}", out[19_999]);
        // a name that already looks numbered is still skipped
        let out = dedup_names(vec!["a (2).txt".into(), "a.txt".into(), "a.txt".into()]);
        assert_eq!(out, ["a (2).txt", "a.txt", "a (3).txt"]);
    }

    #[test]
    fn dedup_keeps_files_and_folders_apart() {
        let out = dedup_names(vec![
            "docs".into(),
            "docs/readme.txt".into(),
            "Docs/b.txt".into(),
            "docs (2)".into(),
        ]);
        assert_eq!(
            out,
            [
                "docs",
                "docs (2)/readme.txt",
                "docs (2)/b.txt",
                "docs (2) (2)"
            ]
        );
        let out = dedup_names(vec![
            "docs/readme.txt".into(),
            "DOCS".into(),
            "docs/sub/x".into(),
            "docs/sub".into(),
        ]);
        assert_eq!(
            out,
            ["docs/readme.txt", "DOCS (2)", "docs/sub/x", "docs/sub (2)"]
        );
        // deeper: a file in the middle of a later path
        let out = dedup_names(vec!["a/b".into(), "a/b/c/d.txt".into()]);
        assert_eq!(out, ["a/b", "a/b (2)/c/d.txt"]);
        // every result can be extracted: no name is a folder of another
        for names in [
            vec!["x", "x/y", "x/y/z", "X/Y", "x (2)", "x (2)/q"],
            vec!["p/q/r", "p/q", "p", "P/Q/R"],
        ] {
            let out = dedup_names(names.into_iter().map(String::from).collect());
            let files: HashSet<String> = out.iter().map(|s| s.to_lowercase()).collect();
            assert_eq!(files.len(), out.len(), "{out:?}");
            for n in &out {
                let l = n.to_lowercase();
                for p in dir_prefixes(&l) {
                    assert!(!files.contains(p), "{p} is a file and a folder in {out:?}");
                }
            }
        }
    }

    #[test]
    fn content_disposition_encoding() {
        assert_eq!(
            content_disposition("a b.txt"),
            "attachment; filename=\"a b.txt\"; filename*=UTF-8''a%20b.txt"
        );
        assert_eq!(
            content_disposition("报告.pdf"),
            "attachment; filename=\"__.pdf\"; filename*=UTF-8''%E6%8A%A5%E5%91%8A.pdf"
        );
        assert_eq!(
            content_disposition("q\"u\\o'te(1)*.txt"),
            "attachment; filename=\"quo'te(1)*.txt\"; filename*=UTF-8''q%22u%5Co%27te%281%29%2A.txt"
        );
        let cd = content_disposition("\u{1}");
        assert!(cd.starts_with("attachment; filename=\"download\""));
        for c in content_disposition("ü\u{202E}x").chars() {
            assert!(c.is_ascii() && !c.is_ascii_control());
        }
    }

    #[test]
    fn codes_are_six_digits() {
        let mut seen = HashSet::new();
        for _ in 0..2000 {
            let c = random_code().expect("random");
            assert!(is_code(&c), "{c}");
            seen.insert(c);
        }
        assert!(seen.len() > 1900);
        assert!(!is_code("12345"));
        assert!(!is_code("1234567"));
        assert!(!is_code("12345a"));
        assert!(!is_code("１２３４５６"));
    }

    #[test]
    fn tokens_and_base64() {
        assert_eq!(b64url(b""), "");
        assert_eq!(b64url(b"f"), "Zg");
        assert_eq!(b64url(b"fo"), "Zm8");
        assert_eq!(b64url(b"foo"), "Zm9v");
        assert_eq!(b64url(&[0xfb, 0xff]), "-_8");
        let t = new_token().expect("token");
        assert!(is_token(&t.text));
        assert_eq!(t.hash, hash_token(&t.text));
        assert!(!is_token("short"));
        let pw = random_password(16).expect("pw");
        assert_eq!(pw.len(), 16);
        assert!(pw.bytes().all(|b| b.is_ascii_alphanumeric()));
    }

    #[test]
    fn query_decoding() {
        assert_eq!(
            query_param("a=1&u=https%3A%2F%2Fx%2Fd%2F1+2", "u").as_deref(),
            Some("https://x/d/1 2")
        );
        assert_eq!(query_param("u=%E4%B8%AD", "u").as_deref(), Some("中"));
        assert_eq!(query_param("u=%zz", "u"), None);
        assert_eq!(query_param("u=%ff", "u"), None);
        assert_eq!(query_param("x=1", "u"), None);
    }

    #[test]
    fn dos_dates() {
        // 2024-02-29 13:45:58 UTC
        let (t, d) = dos_datetime(1_709_214_358_000);
        assert_eq!(d, ((2024 - 1980) << 9) | (2 << 5) | 29);
        assert_eq!(t, (13 << 11) | (45 << 5) | 29);
        assert_eq!(dos_datetime(0), (0, (1 << 5) | 1));
    }
}
