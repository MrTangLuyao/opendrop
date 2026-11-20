//! Static files embedded at build time (see build.rs).

use http::{HeaderMap, HeaderValue, Method, Response, StatusCode, header};

use crate::body::Body;
use crate::http::{HTML_CSP, plain};

pub struct Asset {
    pub path: &'static str,
    pub body: &'static [u8],
    pub gzip: Option<&'static [u8]>,
    pub ctype: &'static str,
    /// Quoted strong validator of the identity encoding.
    pub etag: &'static str,
    /// Fonts never change under the same name: cache for a year.
    pub immutable: bool,
}

include!(concat!(env!("OUT_DIR"), "/assets.rs"));

pub fn find(path: &str) -> Option<&'static Asset> {
    ASSETS
        .binary_search_by(|a| a.path.cmp(path))
        .ok()
        .and_then(|i| ASSETS.get(i))
}

/// True when the client accepts gzip (q-values of 0 count as refusal).
fn accepts_gzip(headers: &HeaderMap) -> bool {
    headers
        .get_all(header::ACCEPT_ENCODING)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .any(|item| {
            let mut parts = item.split(';');
            let name = parts.next().unwrap_or("").trim();
            if !name.eq_ignore_ascii_case("gzip") {
                return false;
            }
            !parts.any(|p| {
                let p = p.trim();
                p.strip_prefix("q=")
                    .and_then(|q| q.trim().parse::<f32>().ok())
                    .is_some_and(|q| q <= 0.0)
            })
        })
}

fn etag_matches(headers: &HeaderMap, etag: &str) -> bool {
    headers
        .get_all(header::IF_NONE_MATCH)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .any(|t| {
            let t = t.trim();
            t == "*" || t.strip_prefix("W/").unwrap_or(t) == etag
        })
}

/// Serves one embedded asset (GET or HEAD).
pub fn serve(asset: &'static Asset, method: &Method, headers: &HeaderMap) -> Response<Body> {
    let gz = asset.gzip.filter(|_| accepts_gzip(headers));
    let etag_owned;
    let etag: &str = if gz.is_some() {
        // distinct validator per encoding: "abc" -> "abc-gz"
        etag_owned = format!("{}-gz\"", asset.etag.trim_end_matches('"'));
        &etag_owned
    } else {
        asset.etag
    };
    let cache = if asset.immutable {
        "public, max-age=31536000, immutable"
    } else {
        "no-cache"
    };
    let is_html = asset.ctype.starts_with("text/html");

    let mut builder = Response::builder()
        .header(header::CACHE_CONTROL, cache)
        .header(header::CONTENT_TYPE, asset.ctype);
    if let Ok(v) = HeaderValue::from_str(etag) {
        builder = builder.header(header::ETAG, v);
    }
    if asset.gzip.is_some() {
        builder = builder.header(header::VARY, "Accept-Encoding");
    }
    if is_html {
        builder = builder.header(header::CONTENT_SECURITY_POLICY, HTML_CSP);
    }

    if etag_matches(headers, etag) {
        return builder
            .status(StatusCode::NOT_MODIFIED)
            .body(Body::Empty)
            .unwrap_or_else(|_| plain(StatusCode::INTERNAL_SERVER_ERROR, "error"));
    }

    let data = gz.unwrap_or(asset.body);
    if gz.is_some() {
        builder = builder.header(header::CONTENT_ENCODING, "gzip");
    }
    builder = builder.header(header::CONTENT_LENGTH, data.len());
    let body = if method == Method::HEAD {
        Body::Empty
    } else {
        Body::full(bytes::Bytes::from_static(data))
    };
    builder
        .status(StatusCode::OK)
        .body(body)
        .unwrap_or_else(|_| plain(StatusCode::INTERNAL_SERVER_ERROR, "error"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_is_sorted() {
        assert!(ASSETS.windows(2).all(|w| w[0].path < w[1].path));
    }

    #[test]
    fn gzip_negotiation() {
        let mut h = HeaderMap::new();
        assert!(!accepts_gzip(&h));
        h.insert(
            header::ACCEPT_ENCODING,
            HeaderValue::from_static("br, gzip, deflate"),
        );
        assert!(accepts_gzip(&h));
        h.insert(
            header::ACCEPT_ENCODING,
            HeaderValue::from_static("gzip;q=0, br"),
        );
        assert!(!accepts_gzip(&h));
        h.insert(
            header::ACCEPT_ENCODING,
            HeaderValue::from_static("GZIP;q=0.5"),
        );
        assert!(accepts_gzip(&h));
        h.insert(header::ACCEPT_ENCODING, HeaderValue::from_static("x-gzip"));
        assert!(!accepts_gzip(&h));
    }

    #[test]
    fn if_none_match() {
        let mut h = HeaderMap::new();
        h.insert(
            header::IF_NONE_MATCH,
            HeaderValue::from_static("\"a\", W/\"b\""),
        );
        assert!(etag_matches(&h, "\"b\""));
        assert!(etag_matches(&h, "\"a\""));
        assert!(!etag_matches(&h, "\"c\""));
    }
}
