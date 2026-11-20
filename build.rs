//! Embeds every file under `web/` into the binary.
//!
//! Text assets are gzipped at maximum level when that makes them smaller.
//! The generated table is sorted by path so lookups can binary search it.

use std::env;
use std::fmt::Write as _;
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use flate2::Compression;
use flate2::write::GzEncoder;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=web");

    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR is set by cargo"));
    let manifest =
        PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR is set"));
    let root = manifest.join("web");

    let mut files = Vec::new();
    if root.is_dir() {
        walk(&root, &root, &mut files);
    }
    files.sort();

    let mut code = String::from("pub static ASSETS: &[Asset] = &[\n");
    for (i, (rel, abs)) in files.iter().enumerate() {
        let abs_str = abs
            .to_str()
            .unwrap_or_else(|| panic!("asset path is not UTF-8: {}", abs.display()));
        println!("cargo:rerun-if-changed={abs_str}");
        let data = fs::read(abs).unwrap_or_else(|e| panic!("read {abs_str}: {e}"));
        let ext = rel.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase());
        let ext = ext.as_deref().unwrap_or("");

        let gzip = if is_text(ext) {
            let mut enc = GzEncoder::new(Vec::new(), Compression::best());
            enc.write_all(&data).expect("gzip write");
            let gz = enc.finish().expect("gzip finish");
            if gz.len() < data.len() {
                let gz_path = out_dir.join(format!("asset{i}.gz"));
                fs::write(&gz_path, &gz).expect("write gzip asset");
                let gz_str = gz_path.to_str().expect("OUT_DIR is UTF-8").to_owned();
                format!("Some(include_bytes!({gz_str:?}))")
            } else {
                "None".to_owned()
            }
        } else {
            "None".to_owned()
        };

        let etag = format!("\"{:016x}\"", fnv1a(&data));
        let ctype = content_type(ext);
        writeln!(
            code,
            "    Asset {{ path: {rel:?}, body: include_bytes!({abs_str:?}), gzip: {gzip}, \
             ctype: {ctype:?}, etag: {etag:?}, immutable: {} }},",
            is_font(ext)
        )
        .expect("format");
    }
    code.push_str("];\n");
    fs::write(out_dir.join("assets.rs"), code).expect("write assets.rs");
}

fn walk(root: &Path, dir: &Path, out: &mut Vec<(String, PathBuf)>) {
    let entries = fs::read_dir(dir).unwrap_or_else(|e| panic!("read {}: {e}", dir.display()));
    for entry in entries {
        let entry = entry.expect("dir entry");
        let path = entry.path();
        let name = entry.file_name();
        let name = name
            .to_str()
            .unwrap_or_else(|| panic!("non UTF-8 name in {}", dir.display()));
        if name.starts_with('.') {
            continue;
        }
        let kind = entry.file_type().expect("file type");
        if kind.is_dir() {
            walk(root, &path, out);
        } else if kind.is_file() {
            let rel = path
                .strip_prefix(root)
                .expect("path under web/")
                .components()
                .map(|c| c.as_os_str().to_str().expect("UTF-8 component"))
                .collect::<Vec<_>>()
                .join("/");
            out.push((rel, path));
        }
    }
}

fn is_text(ext: &str) -> bool {
    matches!(
        ext,
        "html" | "css" | "js" | "mjs" | "svg" | "json" | "txt" | "map" | "webmanifest" | "xml"
    )
}

fn is_font(ext: &str) -> bool {
    matches!(ext, "woff2" | "woff" | "ttf" | "otf")
}

fn content_type(ext: &str) -> &'static str {
    match ext {
        "html" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "json" | "map" => "application/json",
        "webmanifest" => "application/manifest+json",
        "txt" => "text/plain; charset=utf-8",
        "xml" => "application/xml",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "avif" => "image/avif",
        "ico" => "image/x-icon",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        _ => "application/octet-stream",
    }
}

fn fnv1a(data: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in data {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}
