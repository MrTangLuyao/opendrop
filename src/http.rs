//! HTTP server: accept loop, router, security headers, CSRF and client IP.

use std::convert::Infallible;
use std::future::Future;
use std::net::{IpAddr, SocketAddr};
use std::panic::AssertUnwindSafe;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use http::{HeaderMap, HeaderName, HeaderValue, Method, Request, Response, StatusCode, header};
use http_body::Body as _;
use http_body_util::BodyExt;
use hyper::body::Incoming;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper_util::rt::{TokioIo, TokioTimer};
use hyper_util::server::graceful::GracefulShutdown;
use serde::de::DeserializeOwned;
use serde_json::Value;
use tokio::net::TcpListener;
use tokio::sync::Semaphore;
use tokio::time::Instant;

use crate::body::Body;
use crate::config::Config;
use crate::conn::TimeoutIo;
use crate::ratelimit::{Key, ip_key};
use crate::util::{AppError, is_code};
use crate::{App, admin, assets, auth, download, parcels, qr};

pub type Req = Request<Incoming>;
pub type Resp = Response<Body>;
pub type HResult = Result<Resp, AppError>;

pub const HTML_CSP: &str = "default-src 'self'; script-src 'self'; style-src 'self'; \
img-src 'self' data:; font-src 'self'; connect-src 'self'; object-src 'none'; base-uri 'none'; \
form-action 'self'; frame-ancestors 'none'";
const API_CSP: &str = "default-src 'none'; frame-ancestors 'none'";

pub const MAX_CONNECTIONS: usize = 4096;
pub const HEADER_TIMEOUT: Duration = Duration::from_secs(20);
pub const SHUTDOWN_GRACE: Duration = Duration::from_secs(25);
/// hyper's read and write buffer limit per connection (request heads
/// included); its default of about 400 KB would let 4096 idle connections
/// hold 1.6 GB.
pub const MAX_BUF: usize = 64 * 1024;
/// Limit for ordinary JSON bodies.
pub const JSON_LIMIT: usize = 64 * 1024;
/// Base time allowed to receive a whole JSON body; one more second per
/// `BODY_MIN_RATE` bytes expected.
pub const BODY_TIMEOUT: Duration = Duration::from_secs(30);
pub const BODY_MIN_RATE: u64 = 16 * 1024;
/// Longest gap between two pieces of a JSON body.
pub const BODY_IDLE: Duration = Duration::from_secs(20);

/// Per request facts derived once by the middleware.
pub struct Ctx {
    pub ip: Key,
    pub https: bool,
}

/// Accepts connections until `shutdown` resolves, then drains in-flight
/// requests for up to 25 s.
pub async fn serve<F>(app: Arc<App>, listener: TcpListener, shutdown: F)
where
    F: Future<Output = ()>,
{
    let mut builder = http1::Builder::new();
    builder
        .timer(TokioTimer::new())
        .header_read_timeout(HEADER_TIMEOUT)
        .max_buf_size(MAX_BUF)
        .keep_alive(true);
    let pace = app.cfg.pace;
    // Behind a trusted proxy every peer is the proxy; the per client caps
    // on transfers (keyed by the forwarded address) still apply there.
    let per_peer = !app.cfg.trust_proxy;
    let permits = Arc::new(Semaphore::new(MAX_CONNECTIONS));
    let graceful = GracefulShutdown::new();
    tokio::pin!(shutdown);

    loop {
        let permit = tokio::select! {
            p = Arc::clone(&permits).acquire_owned() => match p {
                Ok(p) => p,
                Err(_) => break,
            },
            () = &mut shutdown => break,
        };
        let accepted = tokio::select! {
            r = listener.accept() => r,
            () = &mut shutdown => break,
        };
        let (stream, peer) = match accepted {
            Ok(v) => v,
            Err(e) => {
                // e.g. out of file descriptors: back off instead of spinning
                crate::log!("warning: accept failed: {e}");
                drop(permit);
                tokio::time::sleep(Duration::from_millis(250)).await;
                continue;
            }
        };
        // one client cannot take every connection slot
        let peer_slot = if per_peer {
            match app.slots.conns.try_acquire(ip_key(peer.ip())) {
                Some(g) => Some(g),
                None => {
                    drop(stream);
                    drop(permit);
                    continue;
                }
            }
        } else {
            None
        };
        let _ = stream.set_nodelay(true);
        let io = TimeoutIo::new(stream, pace.write_stall, pace.read_idle);
        let app = Arc::clone(&app);
        let svc = service_fn(move |req| handle(Arc::clone(&app), peer, req));
        let conn = graceful.watch(builder.serve_connection(TokioIo::new(io), svc));
        tokio::spawn(async move {
            let _ = conn.await;
            drop(peer_slot);
            drop(permit);
        });
    }
    drop(listener);
    tokio::select! {
        () = graceful.shutdown() => {}
        () = tokio::time::sleep(SHUTDOWN_GRACE) => {
            crate::log!("warning: shutdown grace period over, dropping open connections");
        }
    }
}

/// Polls a boxed future, turning a panic into `None` (the connection task
/// survives and the client gets a 500).
struct CatchUnwind<F>(Pin<Box<F>>);

impl<F: Future> Future for CatchUnwind<F> {
    type Output = Option<F::Output>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let inner = self.get_mut().0.as_mut();
        match std::panic::catch_unwind(AssertUnwindSafe(|| inner.poll(cx))) {
            Ok(Poll::Pending) => Poll::Pending,
            Ok(Poll::Ready(v)) => Poll::Ready(Some(v)),
            Err(_) => Poll::Ready(None),
        }
    }
}

async fn handle(app: Arc<App>, peer: SocketAddr, req: Req) -> Result<Resp, Infallible> {
    let api = req.uri().path().starts_with("/api/");
    let https = is_https(&app.cfg, req.headers());
    let resp = CatchUnwind(Box::pin(dispatch(app, peer, req))).await;
    Ok(resp.unwrap_or_else(|| {
        let mut r = error_response(&AppError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
        ));
        finish_headers(&mut r, api, &Method::GET, https);
        r
    }))
}

async fn dispatch(app: Arc<App>, peer: SocketAddr, req: Req) -> Resp {
    let ctx = Ctx {
        ip: ip_key(client_ip(&app.cfg, peer, req.headers())),
        https: is_https(&app.cfg, req.headers()),
    };
    let method = req.method().clone();
    let api = req.uri().path().starts_with("/api/");
    let mut resp = if api {
        if is_state_changing(&method) && !csrf_ok(req.headers(), app.cfg.trust_proxy) {
            error_response(&AppError::new(StatusCode::FORBIDDEN, "csrf"))
        } else {
            match route_api(&app, &ctx, req).await {
                Ok(r) => r,
                Err(e) => error_response(&e),
            }
        }
    } else {
        route_page(&req)
    };
    finish_headers(&mut resp, api, &method, ctx.https);
    resp
}

fn is_state_changing(m: &Method) -> bool {
    matches!(
        *m,
        Method::POST | Method::PUT | Method::PATCH | Method::DELETE
    )
}

fn parse_id(s: &str) -> Result<i64, AppError> {
    match s.parse::<i64>() {
        Ok(v) if v > 0 && v.to_string() == s => Ok(v),
        _ => Err(AppError::not_found()),
    }
}

fn parse_idx(s: &str) -> Result<u32, AppError> {
    match s.parse::<u32>() {
        Ok(v) if v.to_string() == s => Ok(v),
        _ => Err(AppError::not_found()),
    }
}

async fn route_api(app: &Arc<App>, ctx: &Ctx, req: Req) -> HResult {
    let method = req.method().clone();
    let path = req.uri().path().to_owned();
    let rest = path.strip_prefix("/api/").unwrap_or("");
    let segs: Vec<&str> = rest.split('/').collect();
    match (method.as_str(), segs.as_slice()) {
        ("GET", ["info"]) => admin::public_info(app).await,
        ("GET", ["qr"]) => qr_handler(app, ctx, &req).await,

        ("POST", ["auth", "login"]) => auth::login(app, ctx, req).await,
        ("POST", ["auth", "logout"]) => auth::logout(app, ctx, &req).await,
        ("GET", ["auth", "me"]) => auth::me(app, &req).await,
        ("POST", ["auth", "renew"]) => auth::renew(app, ctx, &req).await,

        ("POST", ["parcels"]) => parcels::create(app, ctx, req).await,
        ("PUT", ["parcels", id, "files", idx]) => {
            let (id, idx) = (parse_id(id)?, parse_idx(idx)?);
            parcels::put_file(app, ctx, req, id, idx).await
        }
        ("POST", ["parcels", id, "finish"]) => parcels::finish(app, &req, parse_id(id)?).await,
        ("DELETE", ["parcels", id]) => parcels::cancel(app, &req, parse_id(id)?).await,

        ("POST", ["p", code]) => {
            let code = (*code).to_owned();
            download::info(app, ctx, req, &code).await
        }
        ("POST", ["p", code, "claim"]) => {
            let code = (*code).to_owned();
            download::claim(app, ctx, req, &code).await
        }
        ("GET" | "HEAD", ["dl", token, which]) => {
            download::download(app, ctx, &req, token, which).await
        }

        ("GET", ["me", "parcels"]) => parcels::mine(app, &req).await,
        ("PATCH", ["me", "parcels", code]) => {
            let code = (*code).to_owned();
            parcels::mine_patch(app, req, &code).await
        }
        ("DELETE", ["me", "parcels", code]) => parcels::mine_delete(app, &req, code).await,

        ("GET", ["admin", "overview"]) => admin::overview(app, &req).await,
        ("GET", ["admin", "parcels"]) => admin::parcels(app, &req).await,
        ("GET", ["admin", "accounts"]) => admin::accounts(app, &req).await,
        ("PATCH", ["admin", "parcels", id]) => {
            let id = parse_id(id)?;
            admin::parcel_patch(app, req, id).await
        }
        ("DELETE", ["admin", "parcels", id]) => {
            admin::parcel_delete(app, &req, parse_id(id)?).await
        }
        ("PATCH", ["admin", "accounts", id]) => {
            let id = parse_id(id)?;
            admin::account_patch(app, req, id).await
        }
        ("DELETE", ["admin", "accounts", id]) => {
            admin::account_delete(app, &req, parse_id(id)?).await
        }
        ("PUT", ["admin", "settings"]) => admin::settings_put(app, req).await,
        ("POST", ["admin", "credentials"]) => admin::credentials(app, ctx, req).await,

        (_, segs) if known_api_path(segs) => Err(AppError::new(
            StatusCode::METHOD_NOT_ALLOWED,
            "method_not_allowed",
        )),
        _ => Err(AppError::not_found()),
    }
}

fn known_api_path(segs: &[&str]) -> bool {
    matches!(
        segs,
        ["info"]
            | ["qr"]
            | ["auth", "login" | "logout" | "me" | "renew"]
            | ["parcels"]
            | ["parcels", _]
            | ["parcels", _, "files", _]
            | ["parcels", _, "finish"]
            | ["p", _]
            | ["p", _, "claim"]
            | ["dl", _, _]
            | ["me", "parcels"]
            | ["me", "parcels", _]
            | [
                "admin",
                "overview" | "parcels" | "accounts" | "settings" | "credentials"
            ]
            | ["admin", "parcels" | "accounts", _]
    )
}

/// GET /api/qr: rate limited per client, and rendered on the blocking pool
/// (a few at a time) so that no burst of renders stalls the async workers.
async fn qr_handler(app: &Arc<App>, ctx: &Ctx, req: &Req) -> HResult {
    let text = req
        .uri()
        .query()
        .and_then(|q| crate::util::query_param(q, "u"))
        .filter(|t| !t.is_empty() && t.len() <= qr::MAX_BYTES)
        .ok_or_else(|| AppError::bad_field("u"))?;
    app.limits.qr.take(ctx.ip).map_err(AppError::rate_limited)?;
    let permit = Arc::clone(&app.qr_sem)
        .acquire_owned()
        .await
        .map_err(|e| AppError::internal("qr semaphore", e))?;
    let svg = tokio::task::spawn_blocking(move || {
        let svg = qr::svg(&text);
        drop(permit);
        svg
    })
    .await
    .map_err(|e| AppError::internal("qr task", e))?
    .ok_or_else(|| AppError::bad_field("u"))?;
    let mut resp = Response::new(Body::full(svg));
    let h = resp.headers_mut();
    h.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("image/svg+xml"),
    );
    h.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=86400"),
    );
    Ok(resp)
}

fn route_page(req: &Req) -> Resp {
    let method = req.method();
    let path = req.uri().path();
    if *method != Method::GET && *method != Method::HEAD {
        let mut r = plain(StatusCode::METHOD_NOT_ALLOWED, "method not allowed\n");
        r.headers_mut()
            .insert(header::ALLOW, HeaderValue::from_static("GET, HEAD"));
        return r;
    }
    if path == "/healthz" {
        return plain(StatusCode::OK, "ok");
    }
    let asset = match path {
        "/" => assets::find("index.html"),
        "/admin" => assets::find("admin.html"),
        "/favicon.svg" => assets::find("favicon.svg").or_else(|| assets::find("assets/icon.svg")),
        _ => {
            if let Some(code) = path.strip_prefix("/d/") {
                if is_code(code) {
                    assets::find("index.html")
                } else {
                    None
                }
            } else if let Some(rest) = path.strip_prefix("/assets/") {
                if rest.is_empty() {
                    None
                } else {
                    assets::find(&format!("assets/{rest}"))
                }
            } else {
                None
            }
        }
    };
    match asset {
        Some(a) => assets::serve(a, method, req.headers()),
        None => plain(StatusCode::NOT_FOUND, "not found\n"),
    }
}

fn set_default(h: &mut HeaderMap, name: HeaderName, value: &'static str) {
    if !h.contains_key(&name) {
        h.insert(name, HeaderValue::from_static(value));
    }
}

fn finish_headers(resp: &mut Resp, api: bool, method: &Method, https: bool) {
    let status = resp.status();
    let h = resp.headers_mut();
    set_default(h, header::X_CONTENT_TYPE_OPTIONS, "nosniff");
    set_default(h, header::REFERRER_POLICY, "no-referrer");
    set_default(h, header::X_FRAME_OPTIONS, "DENY");
    set_default(
        h,
        HeaderName::from_static("cross-origin-opener-policy"),
        "same-origin",
    );
    set_default(
        h,
        HeaderName::from_static("cross-origin-resource-policy"),
        "same-origin",
    );
    set_default(
        h,
        HeaderName::from_static("permissions-policy"),
        "camera=(), microphone=(), geolocation=(), interest-cohort=()",
    );
    if api {
        set_default(h, header::CACHE_CONTROL, "no-store");
        set_default(h, header::CONTENT_SECURITY_POLICY, API_CSP);
    }
    if https {
        set_default(h, header::STRICT_TRANSPORT_SECURITY, "max-age=31536000");
    }
    // An upload that failed may still have body bytes in flight: do not try
    // to reuse the connection (and do not drain gigabytes we refused).
    if (*method == Method::PUT && status.as_u16() >= 400) || status == StatusCode::PAYLOAD_TOO_LARGE
    {
        h.insert(header::CONNECTION, HeaderValue::from_static("close"));
    }
}

// ---------------------------------------------------------------------------
// Client facts
// ---------------------------------------------------------------------------

fn parse_ip(s: &str) -> Option<IpAddr> {
    let s = s.trim();
    s.parse::<IpAddr>()
        .ok()
        .or_else(|| s.parse::<SocketAddr>().ok().map(|a| a.ip()))
}

/// Last value of the last occurrence of a comma separated header.
fn last_list_value<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers
        .get_all(name)
        .iter()
        .next_back()
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.rsplit(',').next())
        .map(str::trim)
}

pub fn client_ip(cfg: &Config, peer: SocketAddr, headers: &HeaderMap) -> IpAddr {
    let forwarded = cfg
        .trust_proxy
        .then(|| last_list_value(headers, "x-forwarded-for").and_then(parse_ip))
        .flatten();
    forwarded.unwrap_or_else(|| peer.ip())
}

pub fn is_https(cfg: &Config, headers: &HeaderMap) -> bool {
    cfg.trust_proxy
        && last_list_value(headers, "x-forwarded-proto")
            .is_some_and(|p| p.eq_ignore_ascii_case("https"))
}

fn strip_default_port<'a>(scheme: &str, authority: &'a str) -> &'a str {
    let port = match scheme {
        "http" => ":80",
        "https" => ":443",
        _ => return authority,
    };
    authority.strip_suffix(port).unwrap_or(authority)
}

fn same_authority(scheme: &str, origin_authority: &str, host: &str) -> bool {
    let host = host.split(',').next().unwrap_or("").trim();
    !host.is_empty()
        && strip_default_port(scheme, origin_authority)
            .eq_ignore_ascii_case(strip_default_port(scheme, host))
}

/// Rejects cross site state changing requests (spec section 7).
pub fn csrf_ok(headers: &HeaderMap, trust_proxy: bool) -> bool {
    if let Some(site) = headers.get("sec-fetch-site") {
        let ok = site
            .to_str()
            .is_ok_and(|s| s.eq_ignore_ascii_case("same-origin") || s.eq_ignore_ascii_case("none"));
        if !ok {
            return false;
        }
    }
    let Some(origin) = headers.get(header::ORIGIN) else {
        return true;
    };
    let Ok(origin) = origin.to_str() else {
        return false;
    };
    let Some((scheme, authority)) = origin.split_once("://") else {
        return false; // includes "null"
    };
    if authority.is_empty() || authority.contains('/') {
        return false;
    }
    let matches = |name: &str| {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|h| same_authority(scheme, authority, h))
    };
    matches("host") || (trust_proxy && matches("x-forwarded-host"))
}

// ---------------------------------------------------------------------------
// Request and response helpers
// ---------------------------------------------------------------------------

pub fn header_str<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name).and_then(|v| v.to_str().ok())
}

/// Declared Content-Length, if any (garbage counts as absent; bytes are
/// counted anyway).
pub fn content_length(headers: &HeaderMap) -> Option<u64> {
    header_str(headers, "content-length").and_then(|v| v.trim().parse().ok())
}

fn is_json(headers: &HeaderMap) -> bool {
    header_str(headers, "content-type").is_some_and(|v| {
        v.split(';')
            .next()
            .unwrap_or("")
            .trim()
            .eq_ignore_ascii_case("application/json")
    })
}

/// Reads and parses a JSON body of at most `limit` bytes. With `empty_ok`,
/// a request without a body (and without a content type) parses as `{}`.
pub async fn read_json<T: DeserializeOwned>(
    req: Req,
    limit: usize,
    empty_ok: bool,
) -> Result<T, AppError> {
    let headers = req.headers();
    let no_body = req.body().is_end_stream();
    if !is_json(headers) {
        let bare = empty_ok && no_body && !headers.contains_key(header::CONTENT_TYPE);
        if !bare {
            return Err(AppError::new(
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "bad_content_type",
            ));
        }
    }
    let declared = content_length(headers);
    if declared.is_some_and(|n| n > limit as u64) {
        return Err(AppError::new(StatusCode::PAYLOAD_TOO_LARGE, "too_large"));
    }
    let bytes = read_body(req.into_body(), limit, declared).await?;
    let data: &[u8] = if bytes.is_empty() && empty_ok {
        b"{}"
    } else {
        &bytes
    };
    serde_json::from_slice(data).map_err(|_| AppError::new(StatusCode::BAD_REQUEST, "bad_json"))
}

/// Collects a body of at most `limit` bytes. The sender has to keep a
/// modest pace (no gap over `BODY_IDLE`, the whole body within
/// `BODY_TIMEOUT` plus a second per `BODY_MIN_RATE` bytes), so a slow mobile
/// link gets through while a trickle cannot hold the buffer for long.
async fn read_body(
    mut body: Incoming,
    limit: usize,
    declared: Option<u64>,
) -> Result<Vec<u8>, AppError> {
    let expect = declared.map_or(limit as u64, |n| n.min(limit as u64));
    let allowance = Duration::from_secs(expect / BODY_MIN_RATE);
    let start = Instant::now();
    let deadline = start
        .checked_add(BODY_TIMEOUT.saturating_add(allowance))
        .unwrap_or(start);
    // the exact size when it is declared, so the buffer never doubles
    let capacity = declared.map_or(0, |n| usize::try_from(n).unwrap_or(0).min(limit));
    let mut out = Vec::with_capacity(capacity);
    loop {
        let wait = deadline
            .saturating_duration_since(Instant::now())
            .min(BODY_IDLE);
        match tokio::time::timeout(wait, body.frame()).await {
            Err(_) => return Err(AppError::new(StatusCode::REQUEST_TIMEOUT, "timeout")),
            Ok(None) => return Ok(out),
            Ok(Some(Err(_))) => return Err(AppError::new(StatusCode::BAD_REQUEST, "bad_request")),
            Ok(Some(Ok(frame))) => {
                if let Ok(data) = frame.into_data() {
                    if out.len().saturating_add(data.len()) > limit {
                        return Err(AppError::new(StatusCode::PAYLOAD_TOO_LARGE, "too_large"));
                    }
                    out.extend_from_slice(&data);
                }
            }
        }
    }
}

pub fn plain(status: StatusCode, text: &'static str) -> Resp {
    let mut r = Response::new(Body::full(bytes::Bytes::from_static(text.as_bytes())));
    *r.status_mut() = status;
    r.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/plain; charset=utf-8"),
    );
    r
}

pub fn json_response(status: StatusCode, v: &Value) -> Resp {
    let body = serde_json::to_vec(v).unwrap_or_else(|_| b"{}".to_vec());
    let mut r = Response::new(Body::full(body));
    *r.status_mut() = status;
    r.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    r
}

pub fn ok_json(v: Value) -> HResult {
    Ok(json_response(StatusCode::OK, &v))
}

pub fn error_response(e: &AppError) -> Resp {
    let mut r = json_response(e.status, &e.to_json());
    if let Some(secs) = e.retry_after {
        r.headers_mut()
            .insert(header::RETRY_AFTER, HeaderValue::from(secs));
    }
    r
}

/// Inserts a header whose value was built at runtime.
pub fn put_header(resp: &mut Resp, name: HeaderName, value: &str) -> Result<(), AppError> {
    let v = HeaderValue::from_str(value).map_err(|e| AppError::internal("header value", e))?;
    resp.headers_mut().append(name, v);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(pairs: &[(&'static str, &'static str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.append(*k, HeaderValue::from_static(v));
        }
        h
    }

    #[test]
    fn csrf_rules() {
        assert!(csrf_ok(&headers(&[]), false));
        assert!(csrf_ok(
            &headers(&[("host", "a.example"), ("origin", "https://a.example")]),
            false
        ));
        assert!(csrf_ok(
            &headers(&[("host", "a.example:443"), ("origin", "https://a.example")]),
            false
        ));
        assert!(csrf_ok(
            &headers(&[
                ("host", "A.example:8080"),
                ("origin", "http://a.example:8080")
            ]),
            false
        ));
        assert!(!csrf_ok(
            &headers(&[("host", "a.example"), ("origin", "https://evil.example")]),
            false
        ));
        assert!(!csrf_ok(
            &headers(&[("host", "a.example"), ("origin", "null")]),
            false
        ));
        assert!(!csrf_ok(
            &headers(&[("host", "a.example:8080"), ("origin", "http://a.example")]),
            false
        ));
        assert!(!csrf_ok(
            &headers(&[("sec-fetch-site", "cross-site")]),
            false
        ));
        assert!(!csrf_ok(
            &headers(&[("sec-fetch-site", "same-site")]),
            false
        ));
        assert!(csrf_ok(
            &headers(&[("sec-fetch-site", "same-origin")]),
            false
        ));
        assert!(csrf_ok(&headers(&[("sec-fetch-site", "none")]), false));
        let proxied = headers(&[
            ("host", "127.0.0.1:8080"),
            ("x-forwarded-host", "drop.example"),
            ("origin", "https://drop.example"),
        ]);
        assert!(!csrf_ok(&proxied, false));
        assert!(csrf_ok(&proxied, true));
    }

    #[test]
    fn client_ip_from_proxy() {
        let mut cfg = Config::new("/tmp".into());
        let peer: SocketAddr = "10.0.0.1:5000".parse().expect("addr");
        let h = headers(&[
            ("x-forwarded-for", "1.1.1.1, 2.2.2.2"),
            ("x-forwarded-proto", "https"),
        ]);
        assert_eq!(client_ip(&cfg, peer, &h), peer.ip());
        assert!(!is_https(&cfg, &h));
        cfg.trust_proxy = true;
        assert_eq!(client_ip(&cfg, peer, &h).to_string(), "2.2.2.2");
        assert!(is_https(&cfg, &h));
        let bad = headers(&[("x-forwarded-for", "garbage")]);
        assert_eq!(client_ip(&cfg, peer, &bad), peer.ip());
    }

    #[test]
    fn ids() {
        assert_eq!(parse_id("17").ok(), Some(17));
        assert!(parse_id("017").is_err());
        assert!(parse_id("-1").is_err());
        assert!(parse_id("0").is_err());
        assert!(parse_id("+5").is_err());
        assert_eq!(parse_idx("0").ok(), Some(0));
        assert!(parse_idx("00").is_err());
    }
}
