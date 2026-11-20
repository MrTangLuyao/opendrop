//! End to end tests against the real server on 127.0.0.1:0 with a temporary
//! data directory, using a minimal HTTP/1.1 client over TcpStream.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, SystemTime};

use opendrop::App;
use opendrop::config::Config;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;

const ADMIN_PW: &str = "admin-secret-1";
const MIB: u64 = 1 << 20;

// ---------------------------------------------------------------------------
// Test server
// ---------------------------------------------------------------------------

struct Server {
    addr: SocketAddr,
    app: Arc<App>,
    dir: PathBuf,
    stop: Option<oneshot::Sender<()>>,
}

impl Drop for Server {
    fn drop(&mut self) {
        if let Some(tx) = self.stop.take() {
            let _ = tx.send(());
        }
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn temp_dir() -> PathBuf {
    static N: AtomicU32 = AtomicU32::new(0);
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    let p = std::env::temp_dir().join(format!(
        "od-it-{}-{}-{nanos}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&p).expect("mkdir");
    p
}

async fn start() -> Server {
    start_with(|_| {}).await
}

async fn start_with(f: impl FnOnce(&mut Config)) -> Server {
    start_in(temp_dir(), f).await
}

/// Starts a server on an existing (possibly prepared) data directory.
async fn start_in(dir: PathBuf, f: impl FnOnce(&mut Config)) -> Server {
    let mut cfg = Config::new(dir.clone());
    cfg.admin_password = Some(ADMIN_PW.to_owned());
    cfg.background_sweep = false;
    f(&mut cfg);
    let (app, _) = tokio::task::spawn_blocking(move || App::open(cfg))
        .await
        .expect("join")
        .expect("open app");
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    let (tx, rx) = oneshot::channel::<()>();
    tokio::spawn(opendrop::run(Arc::clone(&app), listener, async {
        let _ = rx.await;
    }));
    Server {
        addr,
        app,
        dir,
        stop: Some(tx),
    }
}

// ---------------------------------------------------------------------------
// Minimal HTTP client
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct Resp {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Resp {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or_else(|_| {
            panic!(
                "not json ({}): {:?}",
                self.status,
                String::from_utf8_lossy(&self.body)
            )
        })
    }

    fn error(&self) -> String {
        self.json()["error"].as_str().unwrap_or("").to_owned()
    }

    fn session(&self) -> Option<String> {
        let c = self.header("set-cookie")?;
        let v = c.strip_prefix("od_sid=")?.split(';').next()?;
        if v.is_empty() {
            None
        } else {
            Some(v.to_owned())
        }
    }
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

fn parse_response(raw: &[u8], head_only: bool) -> Resp {
    let end = find(raw, b"\r\n\r\n")
        .unwrap_or_else(|| panic!("no header end in {:?}", String::from_utf8_lossy(raw)));
    let head = std::str::from_utf8(&raw[..end]).expect("utf8 head");
    let mut lines = head.split("\r\n");
    let status_line = lines.next().expect("status line");
    let status: u16 = status_line
        .split(' ')
        .nth(1)
        .expect("status")
        .parse()
        .expect("code");
    let headers: Vec<(String, String)> = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_owned(), v.trim().to_owned()))
        .collect();
    let rest = &raw[end + 4..];
    let mut resp = Resp {
        status,
        headers,
        body: Vec::new(),
    };
    if head_only {
        assert!(rest.is_empty(), "HEAD response carried a body");
        return resp;
    }
    if resp
        .header("transfer-encoding")
        .is_some_and(|v| v.eq_ignore_ascii_case("chunked"))
    {
        let mut i = 0;
        loop {
            let nl = find(&rest[i..], b"\r\n").expect("chunk size") + i;
            let size =
                usize::from_str_radix(std::str::from_utf8(&rest[i..nl]).expect("hex").trim(), 16)
                    .expect("chunk size hex");
            i = nl + 2;
            if size == 0 {
                break;
            }
            resp.body.extend_from_slice(&rest[i..i + size]);
            i += size + 2;
        }
    } else if let Some(len) = resp
        .header("content-length")
        .and_then(|v| v.parse::<usize>().ok())
    {
        assert!(rest.len() >= len, "short body: {} < {len}", rest.len());
        resp.body = rest[..len].to_vec();
    } else {
        resp.body = rest.to_vec();
    }
    resp
}

#[derive(Clone)]
struct Client {
    addr: SocketAddr,
    cookie: Option<String>,
}

impl Client {
    fn new(s: &Server) -> Self {
        Client {
            addr: s.addr,
            cookie: None,
        }
    }

    fn with_cookie(&self, cookie: Option<String>) -> Self {
        Client {
            addr: self.addr,
            cookie,
        }
    }

    async fn raw(&self, method: &str, path: &str, headers: &[(&str, &str)], body: &[u8]) -> Resp {
        // boxed so the test futures stay small (debug builds inline them)
        Box::pin(self.raw_inner(method, path, headers, body)).await
    }

    async fn raw_inner(
        &self,
        method: &str,
        path: &str,
        headers: &[(&str, &str)],
        body: &[u8],
    ) -> Resp {
        let mut s = TcpStream::connect(self.addr).await.expect("connect");
        let mut req = format!(
            "{method} {path} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n",
            self.addr
        );
        let mut has_len = false;
        for (k, v) in headers {
            if k.eq_ignore_ascii_case("content-length")
                || k.eq_ignore_ascii_case("transfer-encoding")
            {
                has_len = true;
            }
            req.push_str(&format!("{k}: {v}\r\n"));
        }
        if let Some(c) = &self.cookie {
            req.push_str(&format!("Cookie: od_sid={c}\r\n"));
        }
        if !has_len && (!body.is_empty() || method != "GET" && method != "HEAD") {
            req.push_str(&format!("Content-Length: {}\r\n", body.len()));
        }
        req.push_str("\r\n");
        let _ = s.write_all(req.as_bytes()).await;
        let _ = s.write_all(body).await;
        let mut buf = Vec::new();
        let mut chunk = [0u8; 65536];
        loop {
            match s.read(&mut chunk).await {
                Ok(0) | Err(_) => break,
                Ok(n) => buf.extend_from_slice(&chunk[..n]),
            }
        }
        parse_response(&buf, method == "HEAD")
    }

    async fn get(&self, path: &str) -> Resp {
        self.raw("GET", path, &[], b"").await
    }

    async fn send_json(&self, method: &str, path: &str, v: &Value) -> Resp {
        let body = serde_json::to_vec(v).expect("json");
        self.raw(method, path, &[("Content-Type", "application/json")], &body)
            .await
    }

    async fn post(&self, path: &str, v: &Value) -> Resp {
        self.send_json("POST", path, v).await
    }

    async fn login(&self, username: &str, password: &str, admin: bool) -> (Resp, Client) {
        let r = self
            .post(
                "/api/auth/login",
                &json!({"username": username, "password": password, "admin": admin}),
            )
            .await;
        let c = self.with_cookie(r.session());
        (r, c)
    }

    async fn admin(&self) -> Client {
        let (r, c) = self.login("admin", ADMIN_PW, true).await;
        assert_eq!(r.status, 200, "{:?}", r.body);
        c
    }

    async fn put_file(&self, id: i64, idx: u32, token: &str, data: &[u8]) -> Resp {
        self.raw(
            "PUT",
            &format!("/api/parcels/{id}/files/{idx}"),
            &[
                ("X-Upload-Token", token),
                ("Content-Type", "application/octet-stream"),
            ],
            data,
        )
        .await
    }

    async fn finish(&self, id: i64, token: &str) -> Resp {
        self.raw(
            "POST",
            &format!("/api/parcels/{id}/finish"),
            &[("X-Upload-Token", token)],
            b"",
        )
        .await
    }

    /// Creates, uploads and finishes a files parcel; returns (code, id).
    async fn send_files(&self, files: &[(&str, &[u8])], extra: Value) -> (String, i64) {
        Box::pin(self.send_files_inner(files, extra)).await
    }

    async fn send_files_inner(&self, files: &[(&str, &[u8])], extra: Value) -> (String, i64) {
        let decl: Vec<Value> = files
            .iter()
            .map(|(n, d)| json!({"name": n, "size": d.len()}))
            .collect();
        let mut body = json!({"kind": "files", "files": decl});
        if let (Value::Object(m), Value::Object(e)) = (&mut body, extra) {
            m.extend(e);
        }
        let r = self.post("/api/parcels", &body).await;
        assert_eq!(r.status, 201, "{:?}", String::from_utf8_lossy(&r.body));
        let v = r.json();
        let id = v["id"].as_i64().expect("id");
        let token = v["token"].as_str().expect("token").to_owned();
        for (i, (_, data)) in files.iter().enumerate() {
            let r = self.put_file(id, i as u32, &token, data).await;
            assert_eq!(r.status, 204, "{:?}", String::from_utf8_lossy(&r.body));
        }
        let r = self.finish(id, &token).await;
        assert_eq!(r.status, 200);
        (r.json()["code"].as_str().expect("code").to_owned(), id)
    }
}

fn python_zip_listing(data: &[u8]) -> Option<String> {
    let ok = std::process::Command::new("python3")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success());
    if !ok {
        eprintln!("python3 not found, skipping zip validation");
        return None;
    }
    let dir = temp_dir();
    let path = dir.join("dl.zip");
    std::fs::write(&path, data).expect("write zip");
    let out = std::process::Command::new("python3")
        .arg("-c")
        .arg(
            "import zipfile,sys\nz=zipfile.ZipFile(sys.argv[1])\nassert z.testzip() is None\n\
             for i in z.infolist():\n    print(i.filename + '\\t' + str(i.file_size))\n",
        )
        .arg(&path)
        .output()
        .expect("python3");
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    Some(String::from_utf8(out.stdout).expect("utf8"))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn full_files_flow() {
    let s = start().await;
    let c = Client::new(&s);
    let files: [(&str, &[u8]); 3] = [
        ("dir/a.txt", b"hello world"),
        ("空.bin", b""),
        ("b/../../c.txt", b"12345"),
    ];
    let decl: Vec<Value> = files
        .iter()
        .map(|(n, d)| json!({"name": n, "size": d.len()}))
        .collect();
    let r = c
        .post(
            "/api/parcels",
            &json!({"kind":"files","files":decl,"password":"pw1","downloads":2,"expiry_hours":24}),
        )
        .await;
    assert_eq!(r.status, 201);
    let v = r.json();
    let id = v["id"].as_i64().expect("id");
    let token = v["token"].as_str().expect("token").to_owned();

    // wrong token, unknown index
    let bad = c.put_file(id, 0, &"A".repeat(43), b"hello world").await;
    assert_eq!((bad.status, bad.error().as_str()), (403, "bad_token"));
    let r = c.put_file(id, 9, &token, b"x").await;
    assert_eq!(r.status, 404);

    assert_eq!(c.put_file(id, 0, &token, files[0].1).await.status, 204);
    assert_eq!(c.put_file(id, 1, &token, files[1].1).await.status, 204);
    let r = c.finish(id, &token).await;
    assert_eq!((r.status, r.error().as_str()), (409, "incomplete"));
    assert_eq!(c.put_file(id, 2, &token, files[2].1).await.status, 204);
    let r = c.put_file(id, 2, &token, files[2].1).await;
    assert_eq!((r.status, r.error().as_str()), (409, "already_done"));
    let r = c.finish(id, &token).await;
    assert_eq!(r.status, 200);
    let fin = r.json();
    assert_eq!(fin["file_count"], 3);
    assert_eq!(fin["total_bytes"], 16);
    assert_eq!(fin["downloads_left"], 2);
    let code = fin["code"].as_str().expect("code").to_owned();
    assert_eq!(code.len(), 6);
    // the upload token is spent
    assert_eq!(c.finish(id, &token).await.status, 404);

    // info: password rules, does not consume a download
    let path = format!("/api/p/{code}");
    let r = c.post(&path, &json!({})).await;
    assert_eq!((r.status, r.error().as_str()), (401, "password_required"));
    let r = c.post(&path, &json!({"password": "nope"})).await;
    assert_eq!((r.status, r.error().as_str()), (401, "bad_password"));
    let r = c.post(&path, &json!({"password": 12})).await;
    assert_eq!(r.status, 400);
    let r = c.post(&path, &json!({"password": "pw1"})).await;
    assert_eq!(r.status, 200);
    let info = r.json();
    assert_eq!(info["kind"], "files");
    assert_eq!(info["has_password"], true);
    assert_eq!(info["downloads_left"], 2);
    let names: Vec<&str> = info["files"]
        .as_array()
        .expect("files")
        .iter()
        .map(|f| f["name"].as_str().unwrap_or(""))
        .collect();
    assert_eq!(names, ["dir/a.txt", "空.bin", "b/c.txt"]);
    assert_eq!(info["files"][0]["i"], 0);
    assert_eq!(info["files"][0]["size"], 11);
    let r = c.post("/api/p/12345", &json!({})).await;
    assert_eq!((r.status, r.error().as_str()), (400, "bad_code"));

    // claim
    let r = c
        .post(&format!("{path}/claim"), &json!({"password": "pw1"}))
        .await;
    assert_eq!(r.status, 200);
    let cl = r.json();
    assert_eq!(cl["downloads_left"], 1);
    assert!(cl.get("text").is_none());
    let t = cl["token"].as_str().expect("token").to_owned();

    // single file download with headers
    let r = c.get(&format!("/api/dl/{t}/0")).await;
    assert_eq!(r.status, 200);
    assert_eq!(r.body, b"hello world");
    assert_eq!(r.header("content-type"), Some("application/octet-stream"));
    assert_eq!(
        r.header("content-disposition"),
        Some("attachment; filename=\"a.txt\"; filename*=UTF-8''a.txt")
    );
    assert_eq!(r.header("content-length"), Some("11"));
    assert_eq!(r.header("accept-ranges"), Some("bytes"));
    assert_eq!(
        r.header("content-security-policy"),
        Some("sandbox; default-src 'none'")
    );
    assert_eq!(r.header("x-content-type-options"), Some("nosniff"));
    assert_eq!(r.header("cache-control"), Some("no-store"));
    let etag = r.header("etag").expect("etag").to_owned();
    assert!(etag.starts_with(&format!("\"{id}-0-")), "{etag}");

    let r = c.get(&format!("/api/dl/{t}/1")).await;
    assert_eq!(r.status, 200);
    assert!(r.body.is_empty());
    assert_eq!(
        r.header("content-disposition"),
        Some("attachment; filename=\"_.bin\"; filename*=UTF-8''%E7%A9%BA.bin")
    );

    // Range
    let r = c
        .raw(
            "GET",
            &format!("/api/dl/{t}/0"),
            &[("Range", "bytes=6-")],
            b"",
        )
        .await;
    assert_eq!(r.status, 206);
    assert_eq!(r.body, b"world");
    assert_eq!(r.header("content-range"), Some("bytes 6-10/11"));
    let r = c
        .raw(
            "GET",
            &format!("/api/dl/{t}/0"),
            &[("Range", "bytes=-3")],
            b"",
        )
        .await;
    assert_eq!((r.status, r.body.as_slice()), (206, b"rld".as_slice()));
    let r = c
        .raw(
            "GET",
            &format!("/api/dl/{t}/0"),
            &[("Range", "bytes=20-")],
            b"",
        )
        .await;
    assert_eq!(r.status, 416);
    assert_eq!(r.header("content-range"), Some("bytes */11"));
    let r = c
        .raw(
            "GET",
            &format!("/api/dl/{t}/0"),
            &[("Range", "bytes=0-1,3-4")],
            b"",
        )
        .await;
    assert_eq!((r.status, r.body.len()), (200, 11));
    let r = c
        .raw(
            "GET",
            &format!("/api/dl/{t}/0"),
            &[("Range", "bytes=0-1"), ("If-Range", "\"other\"")],
            b"",
        )
        .await;
    assert_eq!(r.status, 200);

    // HEAD
    let r = c.raw("HEAD", &format!("/api/dl/{t}/0"), &[], b"").await;
    assert_eq!(r.status, 200);
    assert_eq!(r.header("content-length"), Some("11"));

    // zip of all files
    let r = c.get(&format!("/api/dl/{t}/zip")).await;
    assert_eq!(r.status, 200);
    assert_eq!(r.header("content-type"), Some("application/zip"));
    assert_eq!(
        r.header("content-disposition"),
        Some(
            format!(
                "attachment; filename=\"opendrop-{code}.zip\"; filename*=UTF-8''opendrop-{code}.zip"
            )
            .as_str()
        )
    );
    let zip = r.body.clone();
    assert_eq!(
        r.header("content-length"),
        Some(zip.len().to_string().as_str())
    );
    if let Some(listing) = python_zip_listing(&zip) {
        assert_eq!(listing, "dir/a.txt\t11\n空.bin\t0\nb/c.txt\t5\n");
    }
    let r = c.raw("HEAD", &format!("/api/dl/{t}/zip"), &[], b"").await;
    assert_eq!(
        r.header("content-length"),
        Some(zip.len().to_string().as_str())
    );
    let r = c
        .raw(
            "GET",
            &format!("/api/dl/{t}/zip"),
            &[("Range", "bytes=0-3")],
            b"",
        )
        .await;
    assert_eq!(
        (r.status, r.body.as_slice()),
        (206, b"PK\x03\x04".as_slice())
    );
    let mid = zip.len() / 2;
    let r = c
        .raw(
            "GET",
            &format!("/api/dl/{t}/zip"),
            &[("Range", &format!("bytes={mid}-"))],
            b"",
        )
        .await;
    assert_eq!(r.body, zip[mid..]);

    // bad targets
    assert_eq!(c.get(&format!("/api/dl/{t}/7")).await.status, 404);
    assert_eq!(c.get(&format!("/api/dl/{t}/01")).await.status, 404);
    let r = c.get(&format!("/api/dl/{}/0", "B".repeat(43))).await;
    assert_eq!((r.status, r.error().as_str()), (403, "bad_token"));
    assert_eq!(c.get("/api/dl/short/0").await.status, 403);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn downloads_run_out() {
    let s = start().await;
    let c = Client::new(&s);
    let (code, _) = c
        .send_files(
            &[("one.txt", b"1"), ("two.txt", b"22")],
            json!({"downloads": 1}),
        )
        .await;
    let r = c.post(&format!("/api/p/{code}/claim"), &json!({})).await;
    assert_eq!(r.status, 200);
    assert_eq!(r.json()["downloads_left"], 0);
    let t = r.json()["token"].as_str().expect("token").to_owned();
    let r = c.post(&format!("/api/p/{code}"), &json!({})).await;
    assert_eq!((r.status, r.error().as_str()), (410, "exhausted"));
    let r = c.post(&format!("/api/p/{code}/claim"), &json!({})).await;
    assert_eq!((r.status, r.error().as_str()), (410, "exhausted"));
    // the claim already made keeps working for its hour
    let r = c.get(&format!("/api/dl/{t}/1")).await;
    assert_eq!((r.status, r.body.as_slice()), (200, b"22".as_slice()));
    // and protects the parcel from the sweeper while it is valid
    opendrop::sweep::run(&s.app, false).await.expect("sweep");
    assert_eq!(c.get(&format!("/api/dl/{t}/0")).await.status, 200);
    // once the claim is gone, the used up parcel is swept
    s.app
        .db
        .call(|c| Ok(c.execute("UPDATE claims SET expires_at = 1", [])?))
        .await
        .expect("db");
    let st = opendrop::sweep::run(&s.app, false).await.expect("sweep");
    assert_eq!(st.parcels, 1);
    assert_eq!(c.get(&format!("/api/dl/{t}/0")).await.status, 403);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn upload_size_is_enforced() {
    let s = start().await;
    let c = Client::new(&s);
    let r = c
        .post(
            "/api/parcels",
            &json!({"kind":"files","files":[{"name":"f","size":10}]}),
        )
        .await;
    let v = r.json();
    let id = v["id"].as_i64().expect("id");
    let token = v["token"].as_str().expect("token").to_owned();
    let path = format!("/api/parcels/{id}/files/0");
    // more bytes than declared
    let r = c.put_file(id, 0, &token, b"0123456789X").await;
    assert_eq!((r.status, r.error().as_str()), (400, "size_mismatch"));
    assert_eq!(r.header("connection"), Some("close"));
    // fewer, chunked (no Content-Length to precheck)
    let r = c
        .raw(
            "PUT",
            &path,
            &[("X-Upload-Token", &token), ("Transfer-Encoding", "chunked")],
            b"5\r\n01234\r\n4\r\n5678\r\n0\r\n\r\n",
        )
        .await;
    assert_eq!((r.status, r.error().as_str()), (400, "size_mismatch"));
    // more, chunked: stopped as soon as the count passes the size
    let r = c
        .raw(
            "PUT",
            &path,
            &[("X-Upload-Token", &token), ("Transfer-Encoding", "chunked")],
            b"8\r\n01234567\r\n8\r\n89abcdef\r\n0\r\n\r\n",
        )
        .await;
    assert_eq!((r.status, r.error().as_str()), (400, "size_mismatch"));
    // right size, chunked
    let r = c
        .raw(
            "PUT",
            &path,
            &[("X-Upload-Token", &token), ("Transfer-Encoding", "chunked")],
            b"5\r\n01234\r\n5\r\n56789\r\n0\r\n\r\n",
        )
        .await;
    assert_eq!(r.status, 204);
    let r = c.finish(id, &token).await;
    let code = r.json()["code"].as_str().expect("code").to_owned();
    let t = c
        .post(&format!("/api/p/{code}/claim"), &json!({}))
        .await
        .json()["token"]
        .as_str()
        .expect("t")
        .to_owned();
    assert_eq!(c.get(&format!("/api/dl/{t}/0")).await.body, b"0123456789");
    // no .part files are left behind
    let leftovers: Vec<_> = std::fs::read_dir(s.app.parcel_dir(id))
        .expect("dir")
        .flatten()
        .map(|e| e.file_name())
        .collect();
    assert_eq!(leftovers, vec![std::ffi::OsString::from("0")]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn upload_validation_and_cancel() {
    let s = start().await;
    let c = Client::new(&s);
    let cases = [
        (json!({"kind":"files","files":[]}), "files"),
        (
            json!({"kind":"files","files":[{"name":"a","size":1}],"downloads":0}),
            "downloads",
        ),
        (
            json!({"kind":"files","files":[{"name":"a","size":1}],"downloads":1000}),
            "downloads",
        ),
        (
            json!({"kind":"files","files":[{"name":"a","size":1}],"expiry_hours":169}),
            "expiry_hours",
        ),
        (
            json!({"kind":"files","files":[{"name":"a","size":1}],"password":"x".repeat(129)}),
            "password",
        ),
        (json!({"kind":"zip"}), "kind"),
        (json!({"kind":"text","text":""}), "text"),
        (json!({"kind":"text"}), "text"),
    ];
    for (body, field) in cases {
        let r = c.post("/api/parcels", &body).await;
        assert_eq!(r.status, 400, "{body}");
        assert_eq!(r.json()["field"], field, "{body}");
    }
    let many: Vec<Value> = (0..1001)
        .map(|i| json!({"name": format!("f{i}"), "size": 0}))
        .collect();
    let r = c
        .post("/api/parcels", &json!({"kind":"files","files":many}))
        .await;
    assert_eq!(r.json()["field"], "files");
    let r = c
        .post(
            "/api/parcels",
            &json!({"kind":"files","files":[{"name":"a","size":-1}]}),
        )
        .await;
    assert_eq!((r.status, r.error().as_str()), (400, "bad_json"));
    let r = c
        .post(
            "/api/parcels",
            &json!({"kind":"files","files":[{"name":"a","size":u64::MAX},{"name":"b","size":2}]}),
        )
        .await;
    assert_eq!((r.status, r.error().as_str()), (413, "too_large"));
    let r = c
        .raw(
            "POST",
            "/api/parcels",
            &[("Content-Type", "text/plain")],
            b"{}",
        )
        .await;
    assert_eq!((r.status, r.error().as_str()), (415, "bad_content_type"));

    // duplicate names are made unique
    let r = c
        .post(
            "/api/parcels",
            &json!({"kind":"files","files":[{"name":"x.txt","size":1},{"name":"X.TXT","size":1}]}),
        )
        .await;
    let v = r.json();
    let id = v["id"].as_i64().expect("id");
    let token = v["token"].as_str().expect("token").to_owned();
    c.put_file(id, 0, &token, b"1").await;
    assert!(s.app.parcel_dir(id).join("0").exists());
    // cancel removes the row and the files
    let r = c
        .raw(
            "DELETE",
            &format!("/api/parcels/{id}"),
            &[("X-Upload-Token", "A".repeat(43).as_str())],
            b"",
        )
        .await;
    assert_eq!(r.status, 403);
    let r = c
        .raw(
            "DELETE",
            &format!("/api/parcels/{id}"),
            &[("X-Upload-Token", &token)],
            b"",
        )
        .await;
    assert_eq!(r.status, 200);
    assert!(!s.app.parcel_dir(id).exists());
    assert_eq!(c.put_file(id, 1, &token, b"1").await.status, 404);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn newer_upload_takes_over() {
    let s = start().await;
    let c = Client::new(&s);
    let r = c
        .post(
            "/api/parcels",
            &json!({"kind":"files","files":[{"name":"f","size":10}]}),
        )
        .await;
    let v = r.json();
    let id = v["id"].as_i64().expect("id");
    let token = v["token"].as_str().expect("token").to_owned();
    // a stalled upload: headers and three bytes, then silence
    let mut stalled = TcpStream::connect(s.addr).await.expect("connect");
    let head = format!(
        "PUT /api/parcels/{id}/files/0 HTTP/1.1\r\nHost: {}\r\nX-Upload-Token: {token}\r\nContent-Length: 10\r\n\r\n012",
        s.addr
    );
    stalled.write_all(head.as_bytes()).await.expect("write");
    tokio::time::sleep(Duration::from_millis(300)).await;
    // the retry wins
    let r = c.put_file(id, 0, &token, b"0123456789").await;
    assert_eq!(r.status, 204);
    let mut buf = Vec::new();
    let _ = tokio::time::timeout(Duration::from_secs(5), stalled.read_to_end(&mut buf)).await;
    let text = String::from_utf8_lossy(&buf);
    assert!(text.starts_with("HTTP/1.1 409"), "{text}");
    assert!(text.contains("superseded"), "{text}");
    assert_eq!(c.finish(id, &token).await.status, 200);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn quota_admission_is_atomic() {
    let s = start().await;
    let c = Client::new(&s);
    let admin = c.admin().await;
    let r = admin
        .send_json(
            "PUT",
            "/api/admin/settings",
            &json!({"storage_mode":"bytes","storage_bytes": 64 * MIB}),
        )
        .await;
    assert_eq!(r.status, 200, "{:?}", r.json());
    assert_eq!(r.json()["storage_bytes"], 64 * MIB);
    let body = json!({"kind":"files","files":[{"name":"big","size": 40 * MIB}]});
    let (a, b) = tokio::join!(c.post("/api/parcels", &body), c.post("/api/parcels", &body));
    let mut statuses = [a.status, b.status];
    statuses.sort_unstable();
    assert_eq!(statuses, [201, 507]);
    let full = if a.status == 507 { a } else { b };
    assert_eq!(full.error(), "quota_full");
    let r = c
        .post(
            "/api/parcels",
            &json!({"kind":"files","files":[{"name":"s","size": 10 * MIB}]}),
        )
        .await;
    assert_eq!(r.status, 201);
    let r = c
        .post(
            "/api/parcels",
            &json!({"kind":"files","files":[{"name":"s","size": 20 * MIB}]}),
        )
        .await;
    assert_eq!((r.status, r.error().as_str()), (507, "quota_full"));
    let info = c.get("/api/info").await.json();
    // two parcels of one file each: sizes plus the per entry overhead
    assert_eq!(
        info["used"],
        50 * MIB + 4 * opendrop::parcels::ENTRY_OVERHEAD
    );
    assert_eq!(info["quota"], 64 * MIB);

    // per upload cap
    let r = admin
        .send_json(
            "PUT",
            "/api/admin/settings",
            &json!({"max_upload_bytes": MIB}),
        )
        .await;
    assert_eq!(r.status, 200);
    let r = c
        .post(
            "/api/parcels",
            &json!({"kind":"files","files":[{"name":"s","size": MIB + 1}]}),
        )
        .await;
    assert_eq!((r.status, r.error().as_str()), (413, "too_large"));
    assert_eq!(r.json()["max"], MIB);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn csrf_is_rejected() {
    let s = start().await;
    let c = Client::new(&s);
    let body = br#"{"username":"eve","password":"x"}"#;
    let r = c
        .raw(
            "POST",
            "/api/auth/login",
            &[
                ("Content-Type", "application/json"),
                ("Origin", "http://evil.example"),
            ],
            body,
        )
        .await;
    assert_eq!((r.status, r.error().as_str()), (403, "csrf"));
    let r = c
        .raw(
            "POST",
            "/api/auth/login",
            &[
                ("Content-Type", "application/json"),
                ("Sec-Fetch-Site", "cross-site"),
            ],
            body,
        )
        .await;
    assert_eq!((r.status, r.error().as_str()), (403, "csrf"));
    let r = c
        .raw(
            "DELETE",
            "/api/me/parcels/123456",
            &[("Sec-Fetch-Site", "same-site")],
            b"",
        )
        .await;
    assert_eq!(r.status, 403);
    let origin = format!("http://{}", s.addr);
    let r = c
        .raw(
            "POST",
            "/api/auth/login",
            &[
                ("Content-Type", "application/json"),
                ("Origin", &origin),
                ("Sec-Fetch-Site", "same-origin"),
            ],
            body,
        )
        .await;
    assert_eq!(r.status, 200);
    // GET is not subject to the check
    let r = c
        .raw(
            "GET",
            "/api/info",
            &[("Origin", "http://evil.example")],
            b"",
        )
        .await;
    assert_eq!(r.status, 200);
    // no CORS answers
    assert!(r.header("access-control-allow-origin").is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn code_misses_are_rate_limited() {
    let s = start().await;
    let c = Client::new(&s);
    for _ in 0..10 {
        let r = c.post("/api/p/000000", &json!({})).await;
        assert_eq!((r.status, r.error().as_str()), (404, "not_found"));
    }
    let r = c.post("/api/p/000000", &json!({})).await;
    assert_eq!((r.status, r.error().as_str()), (429, "rate_limited"));
    let secs = r.json()["retry_after"].as_u64().expect("retry_after");
    assert!((1..=10).contains(&secs));
    assert_eq!(r.header("retry-after"), Some(secs.to_string().as_str()));
    // the brake applies to existing codes too, so probing learns nothing
    let (code, _) = c.send_files(&[("a", b"a")], json!({})).await;
    assert_eq!(
        c.post(&format!("/api/p/{code}"), &json!({})).await.status,
        429
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn parcel_password_failures_are_rate_limited() {
    let s = start().await;
    let c = Client::new(&s);
    let (code, _) = c
        .send_files(&[("a", b"a")], json!({"password": "right"}))
        .await;
    for _ in 0..8 {
        let r = c
            .post(&format!("/api/p/{code}"), &json!({"password": "wrong"}))
            .await;
        assert_eq!(r.error(), "bad_password");
    }
    let r = c
        .post(&format!("/api/p/{code}"), &json!({"password": "right"}))
        .await;
    assert_eq!(r.status, 429);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn accounts_and_sessions() {
    let s = start().await;
    let c = Client::new(&s);
    let r = c
        .post("/api/auth/login", &json!({"username":"a b","password":"x"}))
        .await;
    assert_eq!(
        (r.status, r.json()["field"].as_str()),
        (400, Some("username"))
    );
    let r = c
        .post(
            "/api/auth/login",
            &json!({"username":"alice","password":""}),
        )
        .await;
    assert_eq!(r.json()["field"], "password");

    // first login registers
    let (r, alice) = c.login("  alice ", "pw-alice", false).await;
    assert_eq!(r.status, 200);
    let v = r.json();
    assert_eq!(v["username"], "alice");
    assert_eq!(v["is_admin"], false);
    assert!(v["expires_at"].as_i64().is_some());
    let cookie = r.header("set-cookie").expect("cookie");
    assert!(
        cookie.contains("HttpOnly") && cookie.contains("SameSite=Lax") && cookie.contains("Path=/")
    );
    assert!(cookie.contains("Max-Age=604800"));
    assert!(!cookie.contains("Secure"));
    assert_eq!(alice.get("/api/auth/me").await.json()["username"], "alice");
    // same name, other case, wrong password
    let (r, _) = c.login("ALICE", "nope", false).await;
    assert_eq!((r.status, r.error().as_str()), (401, "bad_credentials"));
    assert!(r.session().is_none());

    // admin flag never registers and refuses non-admins without a session
    let (r, _) = c.login("bob", "pw-bob", true).await;
    assert_eq!((r.status, r.error().as_str()), (401, "bad_credentials"));
    let (r, _) = c.login("alice", "pw-alice", true).await;
    assert_eq!((r.status, r.error().as_str()), (403, "forbidden"));
    assert!(r.header("set-cookie").is_none());

    // admin endpoints
    assert_eq!(c.get("/api/admin/overview").await.status, 401);
    assert_eq!(alice.get("/api/admin/overview").await.error(), "forbidden");
    assert_eq!(
        alice
            .send_json("PUT", "/api/admin/settings", &json!({}))
            .await
            .status,
        403
    );
    let admin = c.admin().await;
    let ov = admin.get("/api/admin/overview").await.json();
    assert_eq!(ov["accounts"], 1);
    assert!(ov["disk"]["total"].as_u64().expect("total") > 0);
    let accounts = admin.get("/api/admin/accounts").await.json();
    let names: Vec<&str> = accounts["accounts"]
        .as_array()
        .expect("list")
        .iter()
        .map(|a| a["username"].as_str().unwrap_or(""))
        .collect();
    assert_eq!(names, ["alice"]); // bob was never created

    // renew
    let r = alice.post("/api/auth/renew", &json!({})).await;
    assert_eq!(r.status, 200);
    assert!(r.session().is_some());
    assert_eq!(
        admin.post("/api/auth/renew", &json!({})).await.error(),
        "never_expires"
    );

    // registration switch
    let r = admin
        .send_json(
            "PUT",
            "/api/admin/settings",
            &json!({"registration": false}),
        )
        .await;
    assert_eq!(r.status, 200);
    let (r, _) = c.login("carol", "pw", false).await;
    assert_eq!((r.status, r.error().as_str()), (403, "registration_closed"));
    assert_eq!(c.get("/api/info").await.json()["registration"], false);

    // perm accounts never expire; un-perm starts a fresh week
    let alice_id = accounts["accounts"][0]["id"].as_i64().expect("id");
    let r = admin
        .send_json(
            "PATCH",
            &format!("/api/admin/accounts/{alice_id}"),
            &json!({"is_perm": true}),
        )
        .await;
    assert_eq!(r.json()["expires_at"], Value::Null);
    assert_eq!(
        alice.post("/api/auth/renew", &json!({})).await.error(),
        "never_expires"
    );
    let r = admin
        .send_json(
            "PATCH",
            &format!("/api/admin/accounts/{alice_id}"),
            &json!({"is_perm": false}),
        )
        .await;
    assert!(r.json()["expires_at"].as_i64().is_some());

    // logout
    let r = alice.post("/api/auth/logout", &json!({})).await;
    assert_eq!(r.status, 200);
    assert!(
        r.header("set-cookie")
            .expect("cookie")
            .contains("Max-Age=0")
    );
    assert_eq!(alice.get("/api/auth/me").await.error(), "unauthorized");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn login_failures_are_rate_limited() {
    let s = start().await;
    let c = Client::new(&s);
    c.login("dave", "right", false).await;
    let attempts: Vec<_> = (0..12).map(|_| c.login("dave", "wrong", false)).collect();
    let mut limited = 0;
    for f in attempts {
        let (r, _) = f.await;
        if r.status == 429 {
            limited += 1;
        }
    }
    assert!(limited >= 4, "{limited}");
    let (r, _) = c.login("dave", "right", false).await;
    assert_eq!(r.status, 429);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn settings_validation() {
    let s = start().await;
    let c = Client::new(&s);
    let admin = c.admin().await;
    for (body, field) in [
        (json!({"storage_percent": 0}), "storage_percent"),
        (json!({"storage_percent": 96}), "storage_percent"),
        (json!({"storage_mode": "all"}), "storage_mode"),
        (json!({"storage_bytes": 1}), "storage_bytes"),
        (
            json!({"max_upload_bytes": u64::MAX / 2}),
            "max_upload_bytes",
        ),
        (json!({"max_expiry_hours": 0}), "max_expiry_hours"),
        (json!({"anon_upload": "no"}), "anon_upload"),
        (json!({"unknown_key": 1}), "unknown_key"),
    ] {
        let r = admin.send_json("PUT", "/api/admin/settings", &body).await;
        assert_eq!((r.status, r.error().as_str()), (400, "bad_field"), "{body}");
        assert_eq!(r.json()["field"], field);
    }
    // nothing was changed by the rejected requests
    let ov = admin.get("/api/admin/overview").await.json();
    assert_eq!(ov["settings"]["storage_percent"], 70);
    // the full settings object round trips
    let r = admin
        .send_json("PUT", "/api/admin/settings", &ov["settings"])
        .await;
    assert_eq!(r.status, 200);
    let r = admin
        .send_json(
            "PUT",
            "/api/admin/settings",
            &json!({"max_expiry_hours": 48, "anon_upload": false}),
        )
        .await;
    assert_eq!(r.json()["max_expiry_hours"], 48);
    let info = c.get("/api/info").await.json();
    assert_eq!(
        (
            info["max_expiry_hours"].as_u64(),
            info["anon_upload"].as_bool()
        ),
        (Some(48), Some(false))
    );
    let r = c
        .post("/api/parcels", &json!({"kind":"text","text":"x"}))
        .await;
    assert_eq!((r.status, r.error().as_str()), (401, "login_required"));
    let r = admin
        .post(
            "/api/parcels",
            &json!({"kind":"text","text":"x","expiry_hours": 49}),
        )
        .await;
    assert_eq!(r.json()["field"], "expiry_hours");
    let r = admin
        .post(
            "/api/parcels",
            &json!({"kind":"text","text":"x","expiry_hours": 48}),
        )
        .await;
    assert_eq!(r.status, 201);
    // persisted across a restart of the app state
    let stored = s
        .app
        .db
        .call(|c| {
            Ok(c.query_row(
                "SELECT value FROM settings WHERE key = 'max_expiry_hours'",
                [],
                |r| r.get::<_, String>(0),
            )?)
        })
        .await
        .expect("db");
    assert_eq!(stored, "48");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn text_parcels() {
    let s = start().await;
    let c = Client::new(&s);
    let text = "héllo\n世界 \"quoted\" \\ end";
    // defaults when the fields are left out: 20 downloads, 48 hours
    let before = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_millis() as i64;
    let d = c
        .post("/api/parcels", &json!({"kind":"text","text": "d"}))
        .await
        .json();
    assert_eq!(d["downloads_left"], 20);
    let left_ms = d["expires_at"].as_i64().expect("expires_at") - before;
    assert!(
        (47 * 3_600_000..=49 * 3_600_000).contains(&left_ms),
        "{left_ms}"
    );
    let r = c
        .post(
            "/api/parcels",
            &json!({"kind":"text","text": text, "downloads": 2}),
        )
        .await;
    assert_eq!(r.status, 201);
    let v = r.json();
    assert_eq!(v["downloads_left"], 2);
    assert!(v["expires_at"].as_i64().is_some());
    let code = v["code"].as_str().expect("code").to_owned();
    let info = c.post(&format!("/api/p/{code}"), &json!({})).await.json();
    assert_eq!(info["kind"], "text");
    assert_eq!(info["files"][0]["name"], "text.txt");
    assert_eq!(info["files"][0]["size"], text.len());
    assert_eq!(info["downloads_left"], 2);
    let r = c.post(&format!("/api/p/{code}/claim"), &json!({})).await;
    let v = r.json();
    assert_eq!(v["text"], text);
    assert_eq!(v["downloads_left"], 1);
    // empty body without content type is accepted as {}
    let r = c
        .raw("POST", &format!("/api/p/{code}/claim"), &[], b"")
        .await;
    assert_eq!(r.status, 200);
    assert_eq!(r.json()["downloads_left"], 0);
    let big = "x".repeat(1 << 20);
    assert_eq!(
        c.post("/api/parcels", &json!({"kind":"text","text": big}))
            .await
            .status,
        201
    );
    let too_big = "x".repeat((1 << 20) + 1);
    assert_eq!(
        c.post("/api/parcels", &json!({"kind":"text","text": too_big}))
            .await
            .json()["field"],
        "text"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn my_parcels_and_admin_edits() {
    let s = start().await;
    let c = Client::new(&s);
    let (_, alice) = c.login("alice", "pw-alice", false).await;
    let (_, bob) = c.login("bob", "pw-bob", false).await;
    let (code, id) = alice
        .send_files(&[("a.txt", b"abc")], json!({"password": "p"}))
        .await;
    let list = alice.get("/api/me/parcels").await.json();
    let row = &list["parcels"][0];
    assert_eq!(row["code"], code.as_str());
    assert_eq!(
        (row["file_count"].as_i64(), row["total_bytes"].as_i64()),
        (Some(1), Some(3))
    );
    assert_eq!(row["has_password"], true);
    assert_eq!(row["kind"], "files");
    assert_eq!(
        bob.get("/api/me/parcels").await.json()["parcels"],
        json!([])
    );
    assert_eq!(c.get("/api/me/parcels").await.status, 401);

    let path = format!("/api/me/parcels/{code}");
    let r = bob
        .send_json("PATCH", &path, &json!({"downloads": 5}))
        .await;
    assert_eq!((r.status, r.error().as_str()), (403, "not_owner"));
    let r = alice
        .send_json(
            "PATCH",
            &path,
            &json!({"downloads": 5, "expiry_hours": 2, "password": ""}),
        )
        .await;
    assert_eq!(r.status, 200);
    let row = r.json();
    assert_eq!(row["downloads_left"], 5);
    assert_eq!(row["has_password"], false);
    let r = alice
        .send_json("PATCH", &path, &json!({"downloads": 1000}))
        .await;
    assert_eq!(r.json()["field"], "downloads");
    let r = alice
        .send_json("PATCH", "/api/me/parcels/000000", &json!({}))
        .await;
    assert_eq!(r.status, 404);

    // admin view and edits
    let admin = c.admin().await;
    let all = admin.get("/api/admin/parcels").await.json();
    let row = &all["parcels"][0];
    assert_eq!(
        (
            row["owner"].as_str(),
            row["state"].as_str(),
            row["id"].as_i64()
        ),
        (Some("alice"), Some("ready"), Some(id))
    );
    let r = admin
        .send_json(
            "PATCH",
            &format!("/api/admin/parcels/{id}"),
            &json!({"expiry_hours": null, "downloads": null, "password": "new"}),
        )
        .await;
    let row = r.json();
    assert_eq!(
        (row["expires_at"].clone(), row["downloads_left"].clone()),
        (Value::Null, Value::Null)
    );
    assert_eq!(row["has_password"], true);
    let r = admin
        .send_json(
            "PATCH",
            &format!("/api/admin/parcels/{id}"),
            &json!({"downloads": 1_000_000}),
        )
        .await;
    assert_eq!(r.json()["field"], "downloads");
    // unlimited downloads stay unlimited after a claim
    let r = c
        .post(&format!("/api/p/{code}/claim"), &json!({"password": "new"}))
        .await;
    assert_eq!(r.json()["downloads_left"], Value::Null);

    // deleting alice deletes her parcels
    let accounts = admin.get("/api/admin/accounts").await.json();
    let alice_row = accounts["accounts"]
        .as_array()
        .expect("list")
        .iter()
        .find(|a| a["username"] == "alice")
        .expect("alice")
        .clone();
    assert_eq!(
        (alice_row["parcels"].as_i64(), alice_row["bytes"].as_i64()),
        (Some(1), Some(3))
    );
    let alice_id = alice_row["id"].as_i64().expect("id");
    let r = admin
        .raw(
            "DELETE",
            &format!("/api/admin/accounts/{alice_id}"),
            &[],
            b"",
        )
        .await;
    assert_eq!(r.status, 200);
    assert!(!s.app.parcel_dir(id).exists());
    assert_eq!(
        c.post(&format!("/api/p/{code}"), &json!({})).await.status,
        404
    );
    assert_eq!(alice.get("/api/auth/me").await.status, 401);

    // bob deletes his own parcel
    let (code, id) = bob.send_files(&[("b", b"b")], json!({})).await;
    assert_eq!(
        alice
            .raw("DELETE", &format!("/api/me/parcels/{code}"), &[], b"")
            .await
            .status,
        401
    );
    let r = bob
        .raw("DELETE", &format!("/api/me/parcels/{code}"), &[], b"")
        .await;
    assert_eq!(r.json()["ok"], true);
    assert!(!s.app.parcel_dir(id).exists());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn admin_credentials_change() {
    let s = start().await;
    let c = Client::new(&s);
    let a1 = c.admin().await;
    let a2 = c.admin().await;
    let r = a1
        .post(
            "/api/admin/credentials",
            &json!({"current": "wrong", "password": "new-password-1"}),
        )
        .await;
    assert_eq!((r.status, r.error().as_str()), (401, "bad_credentials"));
    let r = a1
        .post(
            "/api/admin/credentials",
            &json!({"current": ADMIN_PW, "password": "short"}),
        )
        .await;
    assert_eq!(r.json()["field"], "password");
    let r = a1
        .post("/api/admin/credentials", &json!({"current": ADMIN_PW}))
        .await;
    assert_eq!(r.error(), "no_changes");
    let r = a1
        .post(
            "/api/admin/credentials",
            &json!({"current": ADMIN_PW, "username": "root", "password": "new-password-1"}),
        )
        .await;
    assert_eq!(r.status, 200);
    assert_eq!(r.json()["username"], "root");
    // the other admin session is gone, this one stays
    assert_eq!(a2.get("/api/auth/me").await.status, 401);
    assert_eq!(a1.get("/api/auth/me").await.json()["username"], "root");
    let (r, _) = c.login("root", "new-password-1", true).await;
    assert_eq!(r.status, 200);
    let (r, _) = c.login("admin", ADMIN_PW, true).await;
    assert_eq!(r.status, 401);
}

fn set_old_mtime(path: &Path) {
    let old = SystemTime::now() - Duration::from_secs(7200);
    std::fs::File::open(path)
        .expect("open")
        .set_modified(old)
        .expect("mtime");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn sweeper_removes_expired_parcels_and_files() {
    let s = start().await;
    let c = Client::new(&s);
    let r = c
        .post("/api/parcels", &json!({"kind":"text","text":"bye"}))
        .await;
    let code = r.json()["code"].as_str().expect("code").to_owned();
    let (code2, id2) = c.send_files(&[("keep", b"k")], json!({})).await;
    let id: i64 = s
        .app
        .db
        .call(move |db| {
            Ok(
                db.query_row("SELECT id FROM parcels WHERE code = ?1", [&code], |r| {
                    r.get(0)
                })?,
            )
        })
        .await
        .expect("db");
    assert!(s.app.parcel_dir(id).join("0").exists());
    // a stale upload and an expired parcel
    let r = c
        .post(
            "/api/parcels",
            &json!({"kind":"files","files":[{"name":"x","size":1}]}),
        )
        .await;
    let stale = r.json()["id"].as_i64().expect("id");
    s.app
        .db
        .call(move |db| {
            db.execute("UPDATE parcels SET expires_at = 1 WHERE id = ?1", [id])?;
            db.execute("UPDATE parcels SET touched_at = 1 WHERE id = ?1", [stale])?;
            Ok(())
        })
        .await
        .expect("db");
    // orphans: an old unknown directory goes, a fresh one stays (grace period)
    let old_orphan = s.app.files_dir.join("999999");
    let new_orphan = s.app.files_dir.join("999998");
    std::fs::create_dir_all(&old_orphan).expect("mkdir");
    std::fs::create_dir_all(&new_orphan).expect("mkdir");
    set_old_mtime(&old_orphan);
    // a leftover .part in a finished parcel goes too
    std::fs::write(s.app.parcel_dir(id2).join("0.part"), b"junk").expect("write");

    let st = opendrop::sweep::run(&s.app, true).await.expect("sweep");
    assert_eq!(st.parcels, 2);
    assert_eq!(st.orphans, 1);
    assert!(!s.app.parcel_dir(id).exists());
    assert!(!old_orphan.exists());
    assert!(new_orphan.exists());
    assert!(!s.app.parcel_dir(id2).join("0.part").exists());
    assert!(s.app.parcel_dir(id2).join("0").exists());
    assert_eq!(
        c.post(&format!("/api/p/{code2}"), &json!({})).await.status,
        200
    );
    let left: i64 = s
        .app
        .db
        .call(|db| Ok(db.query_row("SELECT COUNT(*) FROM parcels", [], |r| r.get(0))?))
        .await
        .expect("db");
    assert_eq!(left, 1);

    // expired accounts go, their parcels stay without owner
    let (_, erin) = c.login("erin", "pw-erin", false).await;
    let (code3, _) = erin.send_files(&[("e", b"e")], json!({})).await;
    s.app
        .db
        .call(|db| {
            Ok(db.execute(
                "UPDATE accounts SET expires_at = 1 WHERE username = 'erin'",
                [],
            )?)
        })
        .await
        .expect("db");
    assert_eq!(erin.get("/api/auth/me").await.status, 401);
    let st = opendrop::sweep::run(&s.app, false).await.expect("sweep");
    assert_eq!(st.accounts, 1);
    assert_eq!(
        c.post(&format!("/api/p/{code3}"), &json!({})).await.status,
        200
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn pages_headers_and_misc() {
    let s = start().await;
    let c = Client::new(&s);
    let r = c.get("/healthz").await;
    assert_eq!((r.status, r.body.as_slice()), (200, b"ok".as_slice()));
    for (k, v) in [
        ("x-content-type-options", "nosniff"),
        ("referrer-policy", "no-referrer"),
        ("x-frame-options", "DENY"),
        ("cross-origin-opener-policy", "same-origin"),
        ("cross-origin-resource-policy", "same-origin"),
        (
            "permissions-policy",
            "camera=(), microphone=(), geolocation=(), interest-cohort=()",
        ),
    ] {
        assert_eq!(r.header(k), Some(v), "{k}");
    }
    assert!(r.header("server").is_none());
    assert_eq!(c.get("/nope").await.status, 404);
    assert_eq!(c.get("/d/12345").await.status, 404);
    assert_eq!(c.get("/assets/").await.status, 404);
    assert_eq!(c.raw("POST", "/", &[], b"").await.status, 405);
    let r = c.get("/api/nope").await;
    assert_eq!((r.status, r.error().as_str()), (404, "not_found"));
    let r = c.raw("DELETE", "/api/info", &[], b"").await;
    assert_eq!(r.status, 405);
    let r = c.get("/api/info").await;
    assert_eq!(r.header("cache-control"), Some("no-store"));
    let v = r.json();
    for k in [
        "max_upload",
        "max_expiry_hours",
        "anon_upload",
        "registration",
        "used",
        "quota",
    ] {
        assert!(v.get(k).is_some(), "{k}");
    }
    let r = c
        .get("/api/qr?u=https%3A%2F%2Fx.example%2Fd%2F123456")
        .await;
    assert_eq!(r.status, 200);
    assert_eq!(r.header("content-type"), Some("image/svg+xml"));
    assert_eq!(r.header("cache-control"), Some("public, max-age=86400"));
    assert!(String::from_utf8_lossy(&r.body).contains("crispEdges"));
    assert_eq!(c.get("/api/qr").await.status, 400);
    let long = "a".repeat(513);
    assert_eq!(c.get(&format!("/api/qr?u={long}")).await.status, 400);
    // malformed JSON and oversize bodies
    let r = c
        .raw(
            "POST",
            "/api/auth/login",
            &[("Content-Type", "application/json")],
            b"{nope",
        )
        .await;
    assert_eq!((r.status, r.error().as_str()), (400, "bad_json"));
    let big = vec![b' '; 70 * 1024];
    let r = c
        .raw(
            "POST",
            "/api/auth/login",
            &[("Content-Type", "application/json")],
            &big,
        )
        .await;
    assert_eq!((r.status, r.error().as_str()), (413, "too_large"));
    // the index page exists when the frontend was embedded
    if opendrop::assets::find("index.html").is_some() {
        let r = c
            .raw("GET", "/d/123456", &[("Accept-Encoding", "gzip")], b"")
            .await;
        assert_eq!(r.status, 200);
        assert!(
            r.header("content-security-policy")
                .is_some_and(|v| v.contains("frame-ancestors 'none'"))
        );
        let etag = r.header("etag").expect("etag").to_owned();
        let r = c
            .raw(
                "GET",
                "/",
                &[("Accept-Encoding", "gzip"), ("If-None-Match", &etag)],
                b"",
            )
            .await;
        assert_eq!(r.status, 304);
    }
}

// ---------------------------------------------------------------------------
// Abuse resistance
// ---------------------------------------------------------------------------

/// A TCP connection from a chosen loopback address (all of 127/8 works on
/// Linux), optionally with a tiny receive buffer.
async fn connect_from(addr: SocketAddr, src: [u8; 4], rcvbuf: Option<u32>) -> TcpStream {
    let sock = tokio::net::TcpSocket::new_v4().expect("socket");
    if let Some(n) = rcvbuf {
        sock.set_recv_buffer_size(n).expect("rcvbuf");
    }
    sock.bind(SocketAddr::from((src, 0))).expect("bind");
    sock.connect(addr).await.expect("connect")
}

fn key_of(ip: &str) -> u128 {
    opendrop::ratelimit::ip_key(ip.parse().expect("ip"))
}

/// Reads until the end of the response head; returns the status, the head
/// and the body bytes that came with it.
async fn read_head(s: &mut TcpStream, limit: Duration) -> Option<(u16, String, Vec<u8>)> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    let deadline = tokio::time::Instant::now() + limit;
    loop {
        if let Some(end) = find(&buf, b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&buf[..end]).into_owned();
            let status = head.split(' ').nth(1)?.parse().ok()?;
            return Some((status, head, buf[end + 4..].to_vec()));
        }
        let n = tokio::time::timeout_at(deadline, s.read(&mut chunk))
            .await
            .ok()?
            .ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&chunk[..n]);
    }
}

async fn claim_token(c: &Client, code: &str) -> String {
    let r = c.post(&format!("/api/p/{code}/claim"), &json!({})).await;
    assert_eq!(r.status, 200, "{:?}", String::from_utf8_lossy(&r.body));
    r.json()["token"].as_str().expect("token").to_owned()
}

async fn wait_until(limit: Duration, mut f: impl FnMut() -> bool) -> bool {
    let deadline = tokio::time::Instant::now() + limit;
    while tokio::time::Instant::now() < deadline {
        if f() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    f()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stalled_readers_are_dropped_and_others_still_get_in() {
    let s = start_with(|c| c.pace.write_stall = Duration::from_secs(1)).await;
    let c = Client::new(&s);
    let size = 32 * MIB as usize;
    let data = vec![7u8; size];
    let (code, _) = c.send_files(&[("big.bin", &data)], json!({})).await;
    let t = claim_token(&c, &code).await;
    let req = format!("GET /api/dl/{t}/0 HTTP/1.1\r\nHost: x\r\n\r\n");

    // 40 downloads that never read, from one client
    let mut stalled = Vec::new();
    for _ in 0..40 {
        let mut sock = connect_from(s.addr, [127, 0, 0, 2], Some(4096)).await;
        sock.write_all(req.as_bytes()).await.expect("write");
        stalled.push(sock);
    }
    let attacker = key_of("127.0.0.2");
    let cap = opendrop::ratelimit::DOWNLOADS_PER_CLIENT[0];
    assert!(
        wait_until(Duration::from_secs(3), || s
            .app
            .slots
            .downloads
            .active(attacker)
            == cap)
        .await
    );
    // the ones over the cap were refused at once
    let mut statuses = Vec::new();
    for sock in &mut stalled {
        let mut head = [0u8; 12];
        let got = tokio::time::timeout(Duration::from_secs(3), sock.read_exact(&mut head)).await;
        assert!(matches!(got, Ok(Ok(_))), "no response head");
        statuses.push(String::from_utf8_lossy(&head[9..12]).into_owned());
    }
    let ok = statuses.iter().filter(|s| *s == "200").count();
    let refused = statuses.iter().filter(|s| *s == "429").count();
    assert_eq!(
        (ok, refused),
        (cap as usize, 40 - cap as usize),
        "{statuses:?}"
    );

    // everybody else is still served
    let r = c.get("/healthz").await;
    assert_eq!(r.status, 200);

    // a slow reader that keeps taking data is never cut off, even though
    // it pauses for most of each second
    let mut slow = connect_from(s.addr, [127, 0, 0, 4], Some(4096)).await;
    slow.write_all(req.as_bytes()).await.expect("write");
    let (status, _, early) = read_head(&mut slow, Duration::from_secs(5))
        .await
        .expect("head");
    assert_eq!(status, 200);
    let mut got = early.len();
    let mut chunk = vec![0u8; 256 * 1024];
    while got < size {
        let burst = tokio::time::Instant::now() + Duration::from_millis(60);
        while got < size {
            match tokio::time::timeout_at(burst, slow.read(&mut chunk)).await {
                Ok(Ok(0)) => panic!("cut off after {got} bytes"),
                Ok(Ok(n)) => got += n,
                Ok(Err(e)) => panic!("cut off after {got} bytes: {e}"),
                Err(_) => break,
            }
        }
        tokio::time::sleep(Duration::from_millis(700)).await;
    }
    assert_eq!(got, size);

    // the stalled ones are dropped after the write stall and free their slots
    assert!(
        wait_until(Duration::from_secs(10), || s
            .app
            .slots
            .downloads
            .active(attacker)
            == 0)
        .await,
        "stalled downloads still hold {} slots",
        s.app.slots.downloads.active(attacker)
    );
    drop(stalled);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn one_peer_cannot_take_every_connection() {
    let s = start().await;
    let cap = opendrop::ratelimit::CONNS_PER_CLIENT[0] as usize;
    let mut socks = Vec::new();
    for _ in 0..cap + 6 {
        socks.push(connect_from(s.addr, [127, 0, 0, 3], None).await);
    }
    // the server closes the extra ones right after accepting them
    tokio::time::sleep(Duration::from_millis(500)).await;
    let mut closed = 0;
    let mut open = 0;
    for sock in &socks {
        let mut b = [0u8; 1];
        match sock.try_read(&mut b) {
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => open += 1,
            _ => closed += 1,
        }
    }
    assert_eq!((open, closed), (cap, 6));
    // another client is not affected
    assert_eq!(Client::new(&s).get("/healthz").await.status, 200);
    drop(socks);
    let peer = key_of("127.0.0.3");
    assert!(
        wait_until(Duration::from_secs(5), || s.app.slots.conns.active(peer)
            == 0)
        .await
    );
}

/// Sends a PUT whose body arrives in `step` byte pieces every `every`.
async fn paced_put(
    s: &Server,
    id: i64,
    token: &str,
    size: usize,
    step: usize,
    every: Duration,
) -> (u16, String) {
    let mut sock = TcpStream::connect(s.addr).await.expect("connect");
    let head = format!(
        "PUT /api/parcels/{id}/files/0 HTTP/1.1\r\nHost: x\r\nX-Upload-Token: {token}\r\n\
         Content-Length: {size}\r\nConnection: close\r\n\r\n"
    );
    sock.write_all(head.as_bytes()).await.expect("write");
    let (mut rd, mut wr) = sock.into_split();
    let writer = tokio::spawn(async move {
        let mut sent = 0;
        while sent < size {
            let n = step.min(size - sent);
            if wr.write_all(&vec![b'u'; n]).await.is_err() {
                break;
            }
            sent += n;
            tokio::time::sleep(every).await;
        }
        wr
    });
    let mut out = Vec::new();
    let _ = tokio::time::timeout(Duration::from_secs(20), rd.read_to_end(&mut out)).await;
    writer.abort();
    let text = String::from_utf8_lossy(&out).into_owned();
    let status = text
        .split(' ')
        .nth(1)
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    (status, text)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn uploads_need_a_minimum_pace_but_slow_ones_finish() {
    let s = start_with(|c| {
        c.pace.upload_window = Duration::from_secs(1);
        c.pace.upload_min_bytes = 2048;
    })
    .await;
    let c = Client::new(&s);
    let create = |size: usize| {
        let c = c.clone();
        async move {
            let v = c
                .post(
                    "/api/parcels",
                    &json!({"kind":"files","files":[{"name":"f","size":size}]}),
                )
                .await
                .json();
            (
                v["id"].as_i64().expect("id"),
                v["token"].as_str().expect("token").to_owned(),
            )
        }
    };
    // one byte every 200 ms: alive by the old idle rule, too slow now
    let (id, token) = create(100).await;
    let (status, text) = paced_put(&s, id, &token, 100, 1, Duration::from_millis(200)).await;
    assert_eq!(status, 408, "{text}");
    assert!(text.contains("too_slow"), "{text}");
    // 16 KiB/s for 3 s is slow but steady: it completes
    let (id2, token2) = create(48 * 1024).await;
    let (status, text) = paced_put(
        &s,
        id2,
        &token2,
        48 * 1024,
        4096,
        Duration::from_millis(250),
    )
    .await;
    assert_eq!(status, 204, "{text}");
    assert_eq!(c.finish(id2, &token2).await.status, 200);

    // a long PUT renews its parcel while the data flows, so that when it
    // breaks off the retry still finds the parcel
    let (id3, token3) = create(1 << 20).await;
    s.app
        .db
        .call(move |db| Ok(db.execute("UPDATE parcels SET touched_at = 5 WHERE id = ?1", [id3])?))
        .await
        .expect("db");
    {
        let mut sock = TcpStream::connect(s.addr).await.expect("connect");
        let head = format!(
            "PUT /api/parcels/{id3}/files/0 HTTP/1.1\r\nHost: x\r\nX-Upload-Token: {token3}\r\n\
             Content-Length: {}\r\n\r\n",
            1 << 20
        );
        sock.write_all(head.as_bytes()).await.expect("write");
        for _ in 0..10 {
            sock.write_all(&[b'v'; 4096]).await.expect("write");
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    }
    let t = touched_at(&s, id3).await;
    assert!(t > 5, "touched_at was not renewed during the upload");
}

async fn touched_at(s: &Server, id: i64) -> i64 {
    s.app
        .db
        .call(move |db| {
            Ok(
                db.query_row("SELECT touched_at FROM parcels WHERE id = ?1", [id], |r| {
                    r.get(0)
                })?,
            )
        })
        .await
        .expect("db")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn quota_charges_entries_and_caps_unfinished_uploads() {
    let s = start_with(|c| c.trust_proxy = true).await;
    let c = Client::new(&s);
    let create_as = |ip: &str, body: Value| {
        let c = c.clone();
        let ip = ip.to_owned();
        async move {
            let ip = ip.as_str();
            let body = serde_json::to_vec(&body).expect("json");
            c.raw(
                "POST",
                "/api/parcels",
                &[
                    ("Content-Type", "application/json"),
                    ("X-Forwarded-For", ip),
                ],
                &body,
            )
            .await
        }
    };
    let one = json!({"kind":"files","files":[{"name":"a","size":1}]});

    // empty files are not free
    let empties: Vec<Value> = (0..1000)
        .map(|i| json!({"name": format!("e{i}"), "size": 0}))
        .collect();
    let r = create_as("198.51.100.9", json!({"kind":"files","files": empties})).await;
    assert_eq!(r.status, 201);
    let info = c.get("/api/info").await.json();
    assert_eq!(info["used"], 1001 * opendrop::parcels::ENTRY_OVERHEAD);

    // three unfinished parcels per client; others are not affected
    for _ in 0..2 {
        assert_eq!(create_as("198.51.100.9", one.clone()).await.status, 201);
    }
    let r = create_as("198.51.100.9", one.clone()).await;
    assert_eq!((r.status, r.error().as_str()), (429, "too_many_uploads"));
    assert_eq!(create_as("198.51.100.10", one.clone()).await.status, 201);

    // IPv6: per /64, then per /56 and /48 however many /64s are used
    for ip in [
        "2001:db8:1:100::1",
        "2001:db8:1:100::2",
        "2001:db8:1:100::3",
    ] {
        assert_eq!(create_as(ip, one.clone()).await.status, 201, "{ip}");
    }
    assert_eq!(
        create_as("2001:db8:1:100::4", one.clone()).await.status,
        429
    );
    for ip in [
        "2001:db8:1:101::1",
        "2001:db8:1:102::1",
        "2001:db8:1:103::1",
    ] {
        assert_eq!(create_as(ip, one.clone()).await.status, 201, "{ip}");
    }
    // the /56 (2001:db8:1:100::/56) is full
    assert_eq!(
        create_as("2001:db8:1:1ff::1", one.clone()).await.status,
        429
    );
    for n in 0..6 {
        let ip = format!("2001:db8:1:{}00::1", n + 2);
        assert_eq!(create_as(&ip, one.clone()).await.status, 201, "{ip}");
    }
    // and now the /48
    assert_eq!(
        create_as("2001:db8:1:9900::1", one.clone()).await.status,
        429
    );
    assert_eq!(create_as("2001:db8:2::1", one.clone()).await.status, 201);

    // a PUT that sends nothing (or a little, then breaks off) does not keep
    // a parcel alive
    let r = create_as(
        "192.0.2.7",
        json!({"kind":"files","files":[{"name":"big","size":100000}]}),
    )
    .await;
    let v = r.json();
    let id = v["id"].as_i64().expect("id");
    let token = v["token"].as_str().expect("token").to_owned();
    s.app
        .db
        .call(move |db| Ok(db.execute("UPDATE parcels SET touched_at = 5 WHERE id = ?1", [id])?))
        .await
        .expect("db");
    {
        let mut sock = TcpStream::connect(s.addr).await.expect("connect");
        let head = format!(
            "PUT /api/parcels/{id}/files/0 HTTP/1.1\r\nHost: x\r\nX-Upload-Token: {token}\r\n\
             Content-Length: 100000\r\n\r\n"
        );
        sock.write_all(head.as_bytes()).await.expect("write");
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(touched_at(&s, id).await, 5);
    {
        let mut sock = TcpStream::connect(s.addr).await.expect("connect");
        let head = format!(
            "PUT /api/parcels/{id}/files/0 HTTP/1.1\r\nHost: x\r\nX-Upload-Token: {token}\r\n\
             Content-Length: 100000\r\n\r\n"
        );
        sock.write_all(head.as_bytes()).await.expect("write");
        sock.write_all(&[1u8; 5000]).await.expect("write");
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(touched_at(&s, id).await, 5);

    // unfinished parcels idle for 15 minutes are swept, younger ones stay
    let now = opendrop::util::now_ms();
    s.app
        .db
        .call(move |db| {
            db.execute(
                "UPDATE parcels SET touched_at = ?1 WHERE id = ?2",
                (now - 14 * 60_000, id),
            )?;
            Ok(())
        })
        .await
        .expect("db");
    opendrop::sweep::run(&s.app, false).await.expect("sweep");
    assert_eq!(touched_at(&s, id).await, now - 14 * 60_000);
    s.app
        .db
        .call(move |db| {
            db.execute(
                "UPDATE parcels SET touched_at = ?1 WHERE id = ?2",
                (now - 16 * 60_000, id),
            )?;
            Ok(())
        })
        .await
        .expect("db");
    let st = opendrop::sweep::run(&s.app, false).await.expect("sweep");
    assert!(st.parcels >= 1);
    let left: i64 = s
        .app
        .db
        .call(move |db| {
            Ok(
                db.query_row("SELECT COUNT(*) FROM parcels WHERE id = ?1", [id], |r| {
                    r.get(0)
                })?,
            )
        })
        .await
        .expect("db");
    assert_eq!(left, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn disk_admission_counts_space_promised_to_unfinished_uploads() {
    let s = start().await;
    let c = Client::new(&s);
    let d = opendrop::disk::disk_info(&s.dir).expect("statvfs");
    let reserve = opendrop::disk::reserve(d.total);
    let headroom = 300 * MIB;
    if d.avail < reserve + 2 * headroom {
        eprintln!("not enough free disk for this test, skipping");
        return;
    }
    let admin = c.admin().await;
    let r = admin
        .send_json(
            "PUT",
            "/api/admin/settings",
            &json!({"storage_mode":"bytes","storage_bytes": d.total,"max_upload_bytes": d.total}),
        )
        .await;
    assert_eq!(r.status, 200, "{:?}", r.json());
    // promise nearly all the free space to one upload; nothing is written yet
    let big = d.avail - reserve - headroom;
    let r = c
        .post(
            "/api/parcels",
            &json!({"kind":"files","files":[{"name":"big","size": big}]}),
        )
        .await;
    assert_eq!(r.status, 201, "{:?}", r.json());
    let v = r.json();
    let (id, token) = (
        v["id"].as_i64().expect("id"),
        v["token"].as_str().expect("token").to_owned(),
    );
    // a second one that only fits if the first is ignored
    let second = json!({"kind":"files","files":[{"name":"b","size": 2 * headroom}]});
    let r = c.post("/api/parcels", &second).await;
    assert_eq!((r.status, r.error().as_str()), (507, "disk_full"));
    // once the promise is gone the space is there again
    let r = c
        .raw(
            "DELETE",
            &format!("/api/parcels/{id}"),
            &[("X-Upload-Token", &token)],
            b"",
        )
        .await;
    assert_eq!(r.status, 200);
    assert_eq!(c.post("/api/parcels", &second).await.status, 201);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn code_probing_is_braked_per_network_and_server_wide() {
    let s = start_with(|c| c.trust_proxy = true).await;
    let c = Client::new(&s);
    let probe = |ip: String, code: &str| {
        let c = c.clone();
        let code = code.to_owned();
        async move {
            c.raw(
                "POST",
                &format!("/api/p/{code}"),
                &[
                    ("Content-Type", "application/json"),
                    ("X-Forwarded-For", ip.as_str()),
                ],
                b"{}",
            )
            .await
            .status
        }
    };
    let miss = |ip: String| probe(ip, "000000");

    // the /64 budget
    for _ in 0..10 {
        assert_eq!(miss("2001:db8:5:1::1".into()).await, 404);
    }
    assert_eq!(miss("2001:db8:5:1::2".into()).await, 429);
    // more /64s of the same /56 share its budget of 30
    for net in ["2001:db8:5:2::1", "2001:db8:5:3::1"] {
        for _ in 0..10 {
            assert_eq!(miss(net.into()).await, 404, "{net}");
        }
    }
    assert_eq!(miss("2001:db8:5:4::1".into()).await, 429);
    // and the /56s of a /48 share its budget of 60
    for net in [
        "2001:db8:5:100::1",
        "2001:db8:5:101::1",
        "2001:db8:5:102::1",
    ] {
        for _ in 0..10 {
            assert_eq!(miss(net.into()).await, 404, "{net}");
        }
    }
    assert_eq!(miss("2001:db8:5:200::1".into()).await, 429);

    // many /48s together drain the server wide budget
    let mut drained = false;
    'outer: for x in 0x100..0x200 {
        for _ in 0..10 {
            if miss(format!("2001:db8:{x:x}::1")).await == 429 {
                drained = true;
                break 'outer;
            }
        }
    }
    assert!(drained, "the server wide miss budget never ran out");
    // now a network gets one try, and a prober is held back right after
    let mut statuses = Vec::new();
    for _ in 0..5 {
        statuses.push(miss("2001:db8:aaaa::1".into()).await);
    }
    assert_eq!(statuses[0], 404, "{statuses:?}");
    assert!(statuses.contains(&429), "{statuses:?}");
    // but receivers with the right code are not locked out
    let r = c
        .raw(
            "POST",
            "/api/parcels",
            &[
                ("Content-Type", "application/json"),
                ("X-Forwarded-For", "192.0.2.50"),
            ],
            br#"{"kind":"text","text":"hello"}"#,
        )
        .await;
    assert_eq!(r.status, 201);
    let code = r.json()["code"].as_str().expect("code").to_owned();
    for _ in 0..3 {
        assert_eq!(probe("2001:db8:bbbb::1".into(), &code).await, 200);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn zip_download_survives_deletion_of_its_parcel() {
    let s = start().await;
    let c = Client::new(&s);
    let a = vec![1u8; 6 * MIB as usize];
    let b = vec![2u8; 6 * MIB as usize];
    let (code, id) = c
        .send_files(&[("a.bin", &a), ("dir/b.bin", &b)], json!({}))
        .await;
    let t = claim_token(&c, &code).await;
    let mut sock = connect_from(s.addr, [127, 0, 0, 1], Some(4096)).await;
    let req = format!("GET /api/dl/{t}/zip HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n");
    sock.write_all(req.as_bytes()).await.expect("write");
    let (status, head, mut body) = read_head(&mut sock, Duration::from_secs(5))
        .await
        .expect("head");
    assert_eq!(status, 200);
    let len: usize = head
        .lines()
        .find_map(|l| l.strip_prefix("content-length: "))
        .and_then(|v| v.trim().parse().ok())
        .expect("content-length");
    // take the first megabyte, then the admin deletes the parcel
    let mut chunk = vec![0u8; 64 * 1024];
    while body.len() < MIB as usize {
        let n = sock.read(&mut chunk).await.expect("read");
        assert!(n > 0);
        body.extend_from_slice(&chunk[..n]);
    }
    let admin = c.admin().await;
    let r = admin
        .raw("DELETE", &format!("/api/admin/parcels/{id}"), &[], b"")
        .await;
    assert_eq!(r.status, 200);
    // the claim is gone for new requests, the stream goes on
    assert_eq!(c.get(&format!("/api/dl/{t}/zip")).await.status, 403);
    assert!(s.app.parcel_dir(id).exists());
    sock.read_to_end(&mut body).await.expect("rest");
    assert_eq!(body.len(), len);
    if let Some(listing) = python_zip_listing(&body) {
        assert_eq!(
            listing,
            format!("a.bin\t{}\ndir/b.bin\t{}\n", a.len(), b.len())
        );
    }
    // and the files go once the last stream has ended
    let dir = s.app.parcel_dir(id);
    assert!(wait_until(Duration::from_secs(5), || !dir.exists()).await);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn admin_lists_are_paged() {
    let s = start().await;
    let c = Client::new(&s);
    let admin = c.admin().await;
    for i in 0..5 {
        let r = c
            .post(
                "/api/parcels",
                &json!({"kind":"text","text": format!("t{i}")}),
            )
            .await;
        assert_eq!(r.status, 201);
    }
    let mut seen = Vec::new();
    let mut url = "/api/admin/parcels?limit=2".to_owned();
    loop {
        let v = admin.get(&url).await.json();
        let rows = v["parcels"].as_array().expect("rows").clone();
        assert!(rows.len() <= 2);
        seen.extend(rows.iter().map(|r| r["id"].as_i64().expect("id")));
        match v["next"].as_i64() {
            Some(n) => url = format!("/api/admin/parcels?limit=2&before={n}"),
            None => break,
        }
    }
    let mut sorted = seen.clone();
    sorted.sort_unstable_by(|a, b| b.cmp(a));
    sorted.dedup();
    assert_eq!(seen, sorted, "newest first, no repeats");
    assert_eq!(seen.len(), 5);
    assert_eq!(admin.get("/api/admin/parcels?limit=0").await.status, 400);
    let all = admin.get("/api/admin/parcels").await.json();
    assert_eq!(all["parcels"].as_array().map(Vec::len), Some(5));
    assert!(all["next"].is_null());

    for name in ["u1", "u2", "u3"] {
        c.login(name, "pw", false).await;
    }
    let first = admin.get("/api/admin/accounts?limit=2").await.json();
    assert_eq!(first["accounts"].as_array().map(Vec::len), Some(2));
    let next = first["next"].as_i64().expect("next");
    let rest = admin
        .get(&format!("/api/admin/accounts?limit=2&after={next}"))
        .await
        .json();
    assert_eq!(rest["accounts"][0]["username"], "u3");
    assert!(rest["next"].is_null());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn qr_codes_are_rate_limited() {
    let s = start().await;
    let c = Client::new(&s);
    let url = "/api/qr?u=https%3A%2F%2Fx.example%2Fd%2F123456";
    for _ in 0..20 {
        assert_eq!(c.get(url).await.status, 200);
    }
    let r = c.get(url).await;
    assert_eq!((r.status, r.error().as_str()), (429, "rate_limited"));
    // bytes, not characters, are counted
    let cjk = "%E7%95%8C".repeat(171); // 513 bytes
    assert_eq!(c.get(&format!("/api/qr?u={cjk}")).await.status, 400);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn secure_sessions_use_a_host_only_cookie() {
    let s = start_with(|c| c.trust_proxy = true).await;
    let c = Client::new(&s);
    let https = [
        ("Content-Type", "application/json"),
        ("X-Forwarded-Proto", "https"),
    ];
    let r = c
        .raw(
            "POST",
            "/api/auth/login",
            &https,
            br#"{"username":"carol","password":"pw-carol"}"#,
        )
        .await;
    assert_eq!(r.status, 200);
    let set = r.header("set-cookie").expect("cookie").to_owned();
    assert!(set.starts_with("__Host-od_sid="), "{set}");
    assert!(set.contains("; Secure") && set.contains("Path=/"), "{set}");
    assert!(!set.to_ascii_lowercase().contains("domain"), "{set}");
    let token = set
        .strip_prefix("__Host-od_sid=")
        .and_then(|v| v.split(';').next())
        .expect("value")
        .to_owned();
    let me = |cookie: String, proto: &'static str| {
        let c = c.clone();
        async move {
            c.raw(
                "GET",
                "/api/auth/me",
                &[("Cookie", cookie.as_str()), ("X-Forwarded-Proto", proto)],
                b"",
            )
            .await
            .status
        }
    };
    assert_eq!(me(format!("__Host-od_sid={token}"), "https").await, 200);
    // a plain cookie of the same name (as a sibling site could plant) is
    // not read over https
    assert_eq!(me(format!("od_sid={token}"), "https").await, 401);
    // over http two different values are ambiguous: neither is trusted
    let (r, _) = c.login("dan", "pw-dan", false).await;
    let plain = r.session().expect("session");
    assert_eq!(me(format!("od_sid={plain}"), "http").await, 200);
    let other = "A".repeat(43);
    assert_eq!(
        me(format!("od_sid={other}; od_sid={plain}"), "http").await,
        401
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn guesses_at_one_account_share_a_budget() {
    let s = start_with(|c| c.trust_proxy = true).await;
    let c = Client::new(&s);
    let login_as = |ip: String, password: &'static str| {
        let c = c.clone();
        async move {
            let body = json!({"username": "victim", "password": password});
            c.raw(
                "POST",
                "/api/auth/login",
                &[
                    ("Content-Type", "application/json"),
                    ("X-Forwarded-For", ip.as_str()),
                ],
                &serde_json::to_vec(&body).expect("json"),
            )
            .await
            .status
        }
    };
    assert_eq!(login_as("192.0.2.1".into(), "right-one").await, 200);
    // ten guesses from ten addresses use up the account's budget
    for i in 0..10 {
        assert_eq!(login_as(format!("203.0.113.{i}"), "wrong").await, 401);
    }
    // a client that already guessed gets one more try, then waits
    assert_eq!(login_as("203.0.113.0".into(), "wrong").await, 401);
    assert_eq!(login_as("203.0.113.0".into(), "wrong").await, 429);
    // the owner, from an address that has not failed, still gets in
    assert_eq!(login_as("198.51.100.77".into(), "right-one").await, 200);
    assert_eq!(login_as("198.51.100.77".into(), "right-one").await, 200);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_settings_saves_all_apply() {
    let s = start().await;
    let c = Client::new(&s);
    let admin = c.admin().await;
    let changes = [
        json!({"anon_upload": false}),
        json!({"registration": false}),
        json!({"max_expiry_hours": 24}),
        json!({"storage_percent": 50}),
        json!({"site_name": "drop.example"}),
    ];
    let futs: Vec<_> = changes
        .iter()
        .map(|b| {
            let a = admin.clone();
            let b = b.clone();
            tokio::spawn(async move { a.send_json("PUT", "/api/admin/settings", &b).await.status })
        })
        .collect();
    for f in futs {
        assert_eq!(f.await.expect("join"), 200);
    }
    let now = s.app.settings();
    assert!(!now.anon_upload && !now.registration);
    assert_eq!(now.max_expiry_hours, 24);
    assert_eq!(now.storage_percent, 50);
    assert_eq!(now.site_name, "drop.example");
    let total = opendrop::disk::disk_info(&s.dir).expect("statvfs").total;
    let stored = s
        .app
        .db
        .call(move |db| opendrop::settings::Settings::load(db, total))
        .await
        .expect("load");
    assert_eq!(stored, now);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn create_decides_before_reading_a_large_body() {
    let s = start().await;
    let c = Client::new(&s);
    let admin = c.admin().await;
    let limit = opendrop::parcels::CREATE_LIMIT;
    let r = c
        .raw(
            "POST",
            "/api/parcels",
            &[
                ("Content-Type", "application/json"),
                ("Content-Length", &(limit + 1).to_string()),
            ],
            b"",
        )
        .await;
    assert_eq!((r.status, r.error().as_str()), (413, "too_large"));
    let r = admin
        .send_json("PUT", "/api/admin/settings", &json!({"anon_upload": false}))
        .await;
    assert_eq!(r.status, 200);
    // headers only, a big declared body that never comes: refused at once
    let mut sock = TcpStream::connect(s.addr).await.expect("connect");
    sock.write_all(
        b"POST /api/parcels HTTP/1.1\r\nHost: x\r\nContent-Type: application/json\r\n\
          Content-Length: 2000000\r\n\r\n",
    )
    .await
    .expect("write");
    let (status, _, body) = read_head(&mut sock, Duration::from_secs(3))
        .await
        .expect("answered without the body");
    assert_eq!(status, 401);
    assert!(String::from_utf8_lossy(&body).contains("login_required"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn new_database_next_to_old_files_keeps_them() {
    let dir = temp_dir();
    let files = dir.join("files");
    for id in 1..=3 {
        let d = files.join(id.to_string());
        std::fs::create_dir_all(&d).expect("mkdir");
        std::fs::write(d.join("0"), b"old blob").expect("write");
        set_old_mtime(&d);
    }
    let s = start_in(dir, |_| {}).await;
    let c = Client::new(&s);
    // the startup sweep did not take them
    let st = opendrop::sweep::run(&s.app, true).await.expect("sweep");
    assert_eq!(st.orphans, 0);
    for id in 1..=3 {
        assert!(files.join(id.to_string()).join("0").exists());
    }
    // and a new parcel does not move into one of them
    let r = c
        .post("/api/parcels", &json!({"kind":"text","text":"new"}))
        .await;
    assert_eq!(r.status, 201);
    let id: i64 = s
        .app
        .db
        .call(|db| Ok(db.query_row("SELECT MAX(id) FROM parcels", [], |r| r.get(0))?))
        .await
        .expect("db");
    assert!(id > 3, "new parcel got id {id}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn sweeps_work_in_batches_and_keep_totals() {
    let s = start().await;
    // the server answers only after its startup sweep, which must not take
    // the rows below
    assert_eq!(Client::new(&s).get("/healthz").await.status, 200);
    s.app
        .db
        .call(|db| {
            let tx = db.transaction()?;
            for i in 0..1203 {
                tx.execute(
                    "INSERT INTO parcels(code, kind, state, total_bytes, expiry_hours, expires_at, \
                     created_at, touched_at, charged) VALUES (?1, 1, 1, 10, 1, 1, 0, 0, 8202)",
                    [format!("{i:06}")],
                )?;
                tx.execute(
                    "INSERT INTO files(parcel_id, idx, name, size, done) VALUES (?1, 0, 't', 10, 1)",
                    [tx.last_insert_rowid()],
                )?;
            }
            tx.commit()?;
            Ok(())
        })
        .await
        .expect("db");
    let before = s
        .app
        .db
        .call(|db| opendrop::db::usage(db))
        .await
        .expect("usage");
    assert_eq!(before.parcels, 1203);
    assert_eq!(before.used, 1203 * 8202);
    let st = opendrop::sweep::run(&s.app, false).await.expect("sweep");
    assert_eq!(st.parcels, 1203);
    let after = s
        .app
        .db
        .call(|db| opendrop::db::usage(db))
        .await
        .expect("usage");
    assert_eq!(after, opendrop::db::Usage::default());
    // their codes rest for a week before they are handed out again
    let retired: i64 = s
        .app
        .db
        .call(|db| Ok(db.query_row("SELECT COUNT(*) FROM retired_codes", [], |r| r.get(0))?))
        .await
        .expect("db");
    assert_eq!(retired, 1203);
}
