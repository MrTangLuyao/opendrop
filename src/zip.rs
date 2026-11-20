//! STORE zip archives with an exact, precomputed layout.
//!
//! A [`Plan`] holds the offsets and lengths of every part of the archive
//! (local header, file data, central record, end records) but none of their
//! bytes. Headers are generated only when the stream reaches them, at most
//! `META_CHUNK` bytes at a time, so a request costs no memory per entry and
//! one plan can be shared by every download of a parcel. Any byte range is
//! produced by locating its first part with a binary search. CRC32 values
//! come from the upload, so nothing is read twice. Zip64 records are added
//! only where a size or offset does not fit in 32 bits.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError, Weak};

use bytes::Bytes;

use crate::body::Piece;
use crate::util::dos_datetime;

const U32_LIMIT: u64 = 0xFFFF_FFFF;
const VERSION_MADE_BY: u16 = 0x031E; // unix, spec 3.0
const FLAG_UTF8: u16 = 1 << 11;
const EXTERNAL_ATTR: u32 = 0o100_644 << 16; // regular file, rw-r--r--
/// Generated header bytes are sent in pieces of about this size.
const META_CHUNK: usize = 64 * 1024;
const LOCAL_FIXED: u64 = 30;
const CENTRAL_FIXED: u64 = 46;
const EOCD_LEN: u64 = 22;
const EOCD64_LEN: u64 = 56 + 20; // zip64 end record and its locator

pub struct Entry {
    pub name: String,
    pub size: u64,
    pub crc32: u32,
    pub path: PathBuf,
}

struct Planned {
    e: Entry,
    /// Offset of the local header.
    offset: u64,
    local_len: u64,
    /// Offset of the central directory record.
    central_at: u64,
    central_len: u64,
}

impl Planned {
    fn zip64_size(&self) -> bool {
        self.e.size >= U32_LIMIT
    }

    fn zip64_offset(&self) -> bool {
        self.offset >= U32_LIMIT
    }
}

pub struct Plan {
    entries: Vec<Planned>,
    time: u16,
    date: u16,
    cd_offset: u64,
    cd_size: u64,
    end_len: u64,
    pub total: u64,
}

#[derive(Clone, Copy)]
enum Part {
    Local(usize),
    Data(usize),
    Central(usize),
    End,
}

#[derive(Clone, Copy)]
struct Span {
    start: u64,
    len: u64,
    part: Part,
}

fn put16(v: &mut Vec<u8>, x: u16) {
    v.extend_from_slice(&x.to_le_bytes());
}

fn put32(v: &mut Vec<u8>, x: u32) {
    v.extend_from_slice(&x.to_le_bytes());
}

fn put64(v: &mut Vec<u8>, x: u64) {
    v.extend_from_slice(&x.to_le_bytes());
}

fn clamp32(x: u64) -> u32 {
    u32::try_from(x).unwrap_or(u32::MAX)
}

fn name_len(e: &Entry) -> u16 {
    u16::try_from(e.name.len()).unwrap_or(u16::MAX)
}

impl Plan {
    /// Lays the archive out. `None` only if sizes overflow u64 or a name is
    /// longer than 65535 bytes (both impossible for sanitized input).
    pub fn new(entries: Vec<Entry>, mtime_ms: i64) -> Option<Plan> {
        let (time, date) = dos_datetime(mtime_ms);
        let mut planned = Vec::with_capacity(entries.len());
        let mut offset: u64 = 0;
        for e in entries {
            let name = u64::from(u16::try_from(e.name.len()).ok()?);
            let extra = if e.size >= U32_LIMIT { 20 } else { 0 };
            let local_len = LOCAL_FIXED + name + extra;
            let start = offset;
            offset = offset.checked_add(local_len)?.checked_add(e.size)?;
            planned.push(Planned {
                e,
                offset: start,
                local_len,
                central_at: 0,
                central_len: 0,
            });
        }
        let cd_offset = offset;
        let mut pos = cd_offset;
        for p in &mut planned {
            let mut extra = 0;
            if p.zip64_size() {
                extra += 16;
            }
            if p.zip64_offset() {
                extra += 8;
            }
            if extra > 0 {
                extra += 4;
            }
            p.central_len = CENTRAL_FIXED + u64::from(name_len(&p.e)) + extra;
            p.central_at = pos;
            pos = pos.checked_add(p.central_len)?;
        }
        let cd_size = pos - cd_offset;
        let count = planned.len() as u64;
        let need64 = cd_offset >= U32_LIMIT || cd_size >= U32_LIMIT || count >= 0xFFFF;
        let end_len = if need64 {
            EOCD64_LEN + EOCD_LEN
        } else {
            EOCD_LEN
        };
        let total = pos.checked_add(end_len)?;
        Some(Plan {
            entries: planned,
            time,
            date,
            cd_offset,
            cd_size,
            end_len,
            total,
        })
    }

    fn local_header(&self, p: &Planned) -> Vec<u8> {
        let zip64 = p.zip64_size();
        let name = p.e.name.as_bytes();
        let mut h = Vec::with_capacity(usize::try_from(p.local_len).unwrap_or(0));
        put32(&mut h, 0x0403_4b50);
        put16(&mut h, if zip64 { 45 } else { 20 });
        put16(&mut h, FLAG_UTF8);
        put16(&mut h, 0); // stored
        put16(&mut h, self.time);
        put16(&mut h, self.date);
        put32(&mut h, p.e.crc32);
        let size32 = if zip64 { u32::MAX } else { clamp32(p.e.size) };
        put32(&mut h, size32);
        put32(&mut h, size32);
        put16(&mut h, name_len(&p.e));
        put16(&mut h, if zip64 { 20 } else { 0 });
        h.extend_from_slice(name);
        if zip64 {
            put16(&mut h, 0x0001);
            put16(&mut h, 16);
            put64(&mut h, p.e.size);
            put64(&mut h, p.e.size);
        }
        h
    }

    fn central_record(&self, p: &Planned) -> Vec<u8> {
        let (big_size, big_offset) = (p.zip64_size(), p.zip64_offset());
        let zip64 = big_size || big_offset;
        let mut out = Vec::with_capacity(usize::try_from(p.central_len).unwrap_or(0));
        put32(&mut out, 0x0201_4b50);
        put16(&mut out, VERSION_MADE_BY);
        put16(&mut out, if zip64 { 45 } else { 20 });
        put16(&mut out, FLAG_UTF8);
        put16(&mut out, 0);
        put16(&mut out, self.time);
        put16(&mut out, self.date);
        put32(&mut out, p.e.crc32);
        let size32 = if big_size {
            u32::MAX
        } else {
            clamp32(p.e.size)
        };
        put32(&mut out, size32);
        put32(&mut out, size32);
        put16(&mut out, name_len(&p.e));
        let extra_len = (if big_size { 16 } else { 0 }) + (if big_offset { 8 } else { 0 });
        put16(&mut out, if zip64 { 4 + extra_len } else { 0 });
        put16(&mut out, 0); // comment
        put16(&mut out, 0); // disk
        put16(&mut out, 0); // internal attributes
        put32(&mut out, EXTERNAL_ATTR);
        put32(
            &mut out,
            if big_offset {
                u32::MAX
            } else {
                clamp32(p.offset)
            },
        );
        out.extend_from_slice(p.e.name.as_bytes());
        if zip64 {
            put16(&mut out, 0x0001);
            put16(&mut out, extra_len);
            if big_size {
                put64(&mut out, p.e.size);
                put64(&mut out, p.e.size);
            }
            if big_offset {
                put64(&mut out, p.offset);
            }
        }
        out
    }

    fn end_records(&self) -> Vec<u8> {
        let mut tail = Vec::with_capacity(usize::try_from(self.end_len).unwrap_or(0));
        let count = self.entries.len() as u64;
        if self.end_len > EOCD_LEN {
            let eocd64_offset = self.cd_offset.saturating_add(self.cd_size);
            put32(&mut tail, 0x0606_4b50);
            put64(&mut tail, 44);
            put16(&mut tail, VERSION_MADE_BY);
            put16(&mut tail, 45);
            put32(&mut tail, 0);
            put32(&mut tail, 0);
            put64(&mut tail, count);
            put64(&mut tail, count);
            put64(&mut tail, self.cd_size);
            put64(&mut tail, self.cd_offset);
            put32(&mut tail, 0x0706_4b50);
            put32(&mut tail, 0);
            put64(&mut tail, eocd64_offset);
            put32(&mut tail, 1);
        }
        let count16 = u16::try_from(count)
            .ok()
            .filter(|&c| c < 0xFFFF)
            .unwrap_or(0xFFFF);
        put32(&mut tail, 0x0605_4b50);
        put16(&mut tail, 0);
        put16(&mut tail, 0);
        put16(&mut tail, count16);
        put16(&mut tail, count16);
        put32(
            &mut tail,
            if self.cd_size >= U32_LIMIT {
                u32::MAX
            } else {
                clamp32(self.cd_size)
            },
        );
        put32(
            &mut tail,
            if self.cd_offset >= U32_LIMIT {
                u32::MAX
            } else {
                clamp32(self.cd_offset)
            },
        );
        put16(&mut tail, 0);
        tail
    }

    /// The part of the archive that contains byte `pos`.
    fn span_at(&self, pos: u64) -> Option<Span> {
        if pos >= self.total {
            return None;
        }
        if pos < self.cd_offset {
            let i = self
                .entries
                .partition_point(|p| p.offset <= pos)
                .checked_sub(1)?;
            let p = self.entries.get(i)?;
            let data = p.offset.checked_add(p.local_len)?;
            return Some(if pos < data {
                Span {
                    start: p.offset,
                    len: p.local_len,
                    part: Part::Local(i),
                }
            } else {
                Span {
                    start: data,
                    len: p.e.size,
                    part: Part::Data(i),
                }
            });
        }
        let end_at = self.cd_offset.checked_add(self.cd_size)?;
        if pos < end_at {
            let i = self
                .entries
                .partition_point(|p| p.central_at <= pos)
                .checked_sub(1)?;
            let p = self.entries.get(i)?;
            return Some(Span {
                start: p.central_at,
                len: p.central_len,
                part: Part::Central(i),
            });
        }
        Some(Span {
            start: end_at,
            len: self.end_len,
            part: Part::End,
        })
    }

    fn meta_bytes(&self, part: Part) -> Vec<u8> {
        match part {
            Part::Local(i) => self
                .entries
                .get(i)
                .map(|p| self.local_header(p))
                .unwrap_or_default(),
            Part::Central(i) => self
                .entries
                .get(i)
                .map(|p| self.central_record(p))
                .unwrap_or_default(),
            Part::End => self.end_records(),
            Part::Data(_) => Vec::new(),
        }
    }

    /// Number of entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Path and size of every entry's file.
    pub fn files(&self) -> impl Iterator<Item = (&std::path::Path, u64)> {
        self.entries.iter().map(|p| (p.e.path.as_path(), p.e.size))
    }

    /// Body pieces for the byte range `[start, end)` of the archive.
    pub fn pieces(self: &Arc<Self>, start: u64, end: u64) -> Pieces {
        Pieces {
            plan: Arc::clone(self),
            pos: start,
            end: end.min(self.total),
        }
    }
}

/// Lazily produced pieces of a byte range of an archive.
pub struct Pieces {
    plan: Arc<Plan>,
    pos: u64,
    end: u64,
}

impl Iterator for Pieces {
    type Item = Piece;

    fn next(&mut self) -> Option<Piece> {
        let mut buf: Vec<u8> = Vec::new();
        while self.pos < self.end {
            let Some(span) = self.plan.span_at(self.pos) else {
                break;
            };
            // span_at guarantees span.start <= pos < span end, and pos < end
            let a = self.pos.saturating_sub(span.start);
            let b = self
                .end
                .min(span.start.saturating_add(span.len))
                .saturating_sub(span.start);
            if b <= a {
                break;
            }
            if let Part::Data(i) = span.part {
                if !buf.is_empty() {
                    break; // send the headers gathered so far first
                }
                let p = self.plan.entries.get(i)?;
                self.pos = self.pos.saturating_add(b - a);
                return Some(Piece::File {
                    file: None,
                    path: p.e.path.clone(),
                    offset: a,
                    len: b - a,
                });
            }
            let bytes = self.plan.meta_bytes(span.part);
            let (Ok(a8), Ok(b8)) = (usize::try_from(a), usize::try_from(b)) else {
                break;
            };
            let Some(slice) = bytes.get(a8..b8) else {
                break;
            };
            if !buf.is_empty() && buf.len() + slice.len() > META_CHUNK {
                break;
            }
            if buf.capacity() == 0 {
                buf.reserve_exact(META_CHUNK.max(slice.len()));
            }
            buf.extend_from_slice(slice);
            self.pos = self.pos.saturating_add(b - a);
        }
        if buf.is_empty() {
            None
        } else {
            Some(Piece::Mem(Bytes::from(buf)))
        }
    }
}

/// Plans shared by the downloads of a parcel, keyed by (parcel id,
/// created_at). Only weak references are kept: a plan lives as long as a
/// stream uses it.
#[derive(Default)]
pub struct PlanCache {
    inner: Mutex<HashMap<(i64, i64), Weak<Plan>>>,
}

impl PlanCache {
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<(i64, i64), Weak<Plan>>> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn get(&self, key: (i64, i64)) -> Option<Arc<Plan>> {
        self.lock().get(&key).and_then(Weak::upgrade)
    }

    pub fn insert(&self, key: (i64, i64), plan: &Arc<Plan>) {
        let mut m = self.lock();
        m.retain(|_, w| w.strong_count() > 0);
        m.insert(key, Arc::downgrade(plan));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::body::{Body, collect};
    use std::fs::{self, File};
    use std::io::{Seek, SeekFrom, Write};
    use std::path::Path;
    use std::process::Command;

    fn temp_dir(name: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("od-ziptest-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).expect("mkdir");
        p
    }

    fn have_python() -> bool {
        Command::new("python3")
            .arg("--version")
            .output()
            .is_ok_and(|o| o.status.success())
    }

    const CHECK: &str = "import zipfile,sys\n\
z=zipfile.ZipFile(sys.argv[1])\n\
assert z.testzip() is None\n\
for i in z.infolist():\n    print(i.filename + '\\t' + str(i.file_size))\n";

    /// Validates with python's zipfile (CRC of every member) and returns
    /// `name\tsize` lines. `None` when python3 is unavailable.
    fn python_check(zip: &Path) -> Option<String> {
        if !have_python() {
            eprintln!("python3 not found, skipping zip validation");
            return None;
        }
        let out = Command::new("python3")
            .arg("-c")
            .arg(CHECK)
            .arg(zip)
            .output()
            .expect("run python3");
        assert!(
            out.status.success(),
            "python zipfile rejected the archive: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        if Command::new("unzip")
            .arg("-v")
            .output()
            .is_ok_and(|o| o.status.success())
        {
            let u = Command::new("unzip")
                .arg("-tq")
                .arg(zip)
                .output()
                .expect("unzip");
            assert!(
                u.status.success(),
                "unzip -t: {}",
                String::from_utf8_lossy(&u.stdout)
            );
        }
        Some(String::from_utf8(out.stdout).expect("utf8"))
    }

    fn crc(data: &[u8]) -> u32 {
        let mut h = crc32fast::Hasher::new();
        h.update(data);
        h.finalize()
    }

    fn body(plan: &Arc<Plan>, a: u64, b: u64) -> Body {
        Body::stream_from(b - a, Box::new(plan.pieces(a, b)), None)
    }

    fn sample(dir: &Path) -> Vec<Entry> {
        let mut seed: u64 = 0x1234_5678_9abc_def0;
        let mut rnd = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        let specs: [(&str, usize); 5] = [
            ("a.txt", 1000),
            ("dir/空文件.bin", 0),
            ("dir/sub/b.bin", 700_000),
            ("c", 1),
            ("名前 with spaces.dat", 300_123),
        ];
        let mut entries = Vec::new();
        for (i, (name, size)) in specs.iter().enumerate() {
            let data: Vec<u8> = (0..*size).map(|_| (rnd() & 0xff) as u8).collect();
            let path = dir.join(i.to_string());
            fs::write(&path, &data).expect("write");
            entries.push(Entry {
                name: (*name).to_owned(),
                size: data.len() as u64,
                crc32: crc(&data),
                path,
            });
        }
        entries
    }

    #[tokio::test]
    async fn small_archive_is_valid_and_ranges_match() {
        let dir = temp_dir("small");
        let entries = sample(&dir);
        let expect: String = entries
            .iter()
            .map(|e| format!("{}\t{}\n", e.name, e.size))
            .collect();
        let plan = Arc::new(Plan::new(entries, 1_700_000_000_000).expect("plan"));
        let full = collect(body(&plan, 0, plan.total)).await.expect("stream");
        assert_eq!(full.len() as u64, plan.total);
        let zpath = dir.join("out.zip");
        fs::write(&zpath, &full).expect("write zip");
        if let Some(listing) = python_check(&zpath) {
            assert_eq!(listing, expect);
        }

        // random ranges equal the same slice of the full stream
        let mut seed: u64 = 42;
        for _ in 0..300 {
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            let a = (seed >> 11) % plan.total;
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            let b = a + 1 + (seed >> 11) % (plan.total - a);
            let part = collect(body(&plan, a, b)).await.expect("range");
            assert_eq!(
                part.as_slice(),
                &full[a as usize..b as usize],
                "range {a}..{b}"
            );
        }
        // edges
        for (a, b) in [(0, 1), (plan.total - 1, plan.total), (0, plan.total)] {
            let part = collect(body(&plan, a, b)).await.expect("range");
            assert_eq!(part.as_slice(), &full[a as usize..b as usize]);
        }
        let _ = fs::remove_dir_all(&dir);
    }

    /// Writes the archive without reading the (all zero) sparse inputs: data
    /// pieces become holes in the output, except `real` which is copied.
    fn write_sparse(plan: &Arc<Plan>, out: &Path, real: Option<&Path>) {
        let mut f = File::create(out).expect("create");
        let mut pos = 0u64;
        for piece in plan.pieces(0, plan.total) {
            match piece {
                Piece::Mem(b) => {
                    f.seek(SeekFrom::Start(pos)).expect("seek");
                    f.write_all(&b).expect("write");
                    pos += b.len() as u64;
                }
                Piece::File { path, len, .. } => {
                    if real == Some(path.as_path()) {
                        f.seek(SeekFrom::Start(pos)).expect("seek");
                        f.write_all(&fs::read(&path).expect("read")).expect("write");
                    }
                    pos += len;
                }
            }
        }
        assert_eq!(pos, plan.total);
        f.set_len(plan.total).expect("set_len");
    }

    #[test]
    fn zip64_with_sparse_file() {
        let dir = temp_dir("zip64");
        let big_size: u64 = 4 * (1 << 30) + (1 << 29); // 4.5 GiB
        let big = dir.join("0");
        File::create(&big)
            .expect("create")
            .set_len(big_size)
            .expect("sparse");
        let zeros = vec![0u8; 1 << 20];
        let mut h = crc32fast::Hasher::new();
        for _ in 0..big_size / (1 << 20) {
            h.update(&zeros);
        }
        let small = dir.join("1");
        fs::write(&small, b"tail file").expect("write");
        let entries = vec![
            Entry {
                name: "huge.img".into(),
                size: big_size,
                crc32: h.finalize(),
                path: big.clone(),
            },
            Entry {
                name: "after/tail.txt".into(),
                size: 9,
                crc32: crc(b"tail file"),
                path: small.clone(),
            },
        ];
        let plan = Arc::new(Plan::new(entries, 1_700_000_000_000).expect("plan"));
        assert!(plan.total > big_size);
        // the end of the archive is pure metadata (central directory, zip64 records)
        let tail: Vec<Piece> = plan.pieces(plan.total - 200, plan.total).collect();
        assert!(tail.iter().all(|p| matches!(p, Piece::Mem(_))));
        assert_eq!(tail.iter().map(Piece::len).sum::<u64>(), 200);
        let zpath = dir.join("big.zip");
        write_sparse(&plan, &zpath, Some(&small));
        if let Some(listing) = python_check(&zpath) {
            assert_eq!(
                listing,
                format!("huge.img\t{big_size}\nafter/tail.txt\t9\n")
            );
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn many_entries_and_names() {
        // 0xFFFF entries forces the zip64 end record even for tiny data
        let dir = temp_dir("many");
        let path = dir.join("empty");
        fs::write(&path, b"").expect("write");
        let entries: Vec<Entry> = (0..0xFFFFu32)
            .map(|i| Entry {
                name: format!("f{i}"),
                size: 0,
                crc32: 0,
                path: path.clone(),
            })
            .collect();
        let plan = Arc::new(Plan::new(entries, 0).expect("plan"));
        // metadata is produced in bounded pieces
        let pieces: Vec<Piece> = plan.pieces(0, plan.total).collect();
        assert!(pieces.len() > 1);
        assert!(pieces.iter().all(|p| p.len() <= META_CHUNK as u64));
        let zpath = dir.join("many.zip");
        write_sparse(&plan, &zpath, None);
        if let Some(listing) = python_check(&zpath) {
            assert_eq!(listing.lines().count(), 0xFFFF);
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn plans_are_shared_while_used() {
        let cache = PlanCache::default();
        let plan = Arc::new(Plan::new(Vec::new(), 0).expect("plan"));
        cache.insert((1, 2), &plan);
        assert!(cache.get((1, 2)).is_some_and(|p| Arc::ptr_eq(&p, &plan)));
        drop(plan);
        assert!(cache.get((1, 2)).is_none());
    }
}
