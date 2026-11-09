//! The built UI, embedded at compile time.
//!
//! The build script gzips the UI into OUT_DIR, one copy of each file, so a file is in `Gzipped` or in
//! `Raw`, never both. Gzipped files go out as they are to a client that takes gzip (every browser) and
//! are inflated for one that does not. Gzip rather than Brotli: on a local network the 15 to 20% Brotli
//! saves on the wire does not matter, and a Brotli copy beside the gzip one would nearly double what
//! the UI adds to the program, which has to fit a Raspberry Pi Zero.

use std::borrow::Cow;
use std::io::Read;

use axum::body::{Body, Bytes};
use axum::http::header::{
    ACCEPT_ENCODING, CACHE_CONTROL, CONTENT_ENCODING, CONTENT_TYPE, HeaderMap, VARY,
};
use axum::http::{StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use flate2::read::GzDecoder;
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "$OUT_DIR/ui/raw"]
struct Raw;

#[derive(RustEmbed)]
#[folder = "$OUT_DIR/ui/gzip"]
struct Gzipped;

const INDEX: &str = "index.html";

/// Vite names everything it writes to `assets/` after a hash of its content, so those can be cached
/// for good. index.html, and what comes from `public/` such as the icon, keep their names and have to
/// be checked each time.
const HASHED: &str = "public, max-age=31536000, immutable";
const CHECK_EACH_TIME: &str = "no-cache";

enum Stored {
    Raw(Cow<'static, [u8]>),
    Gzipped(Cow<'static, [u8]>),
}

/// GET (and HEAD) for any path no route took: the file, index.html for a path of the single page
/// app, or 404.
pub(crate) async fn serve(uri: Uri, headers: HeaderMap) -> Response {
    let Some((name, stored)) = find(uri.path()) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let response = Response::builder()
        .header(CONTENT_TYPE, content_type(name))
        .header(CACHE_CONTROL, cache_control(name));
    let built = match stored {
        Stored::Raw(data) => response.body(body(data)),
        Stored::Gzipped(data) if accepts_gzip(&headers) => response
            .header(CONTENT_ENCODING, "gzip")
            .header(VARY, "accept-encoding")
            .body(body(data)),
        Stored::Gzipped(data) => {
            let mut inflated = Vec::new();
            if let Err(e) = GzDecoder::new(data.as_ref()).read_to_end(&mut inflated) {
                tracing::error!("the embedded {name} does not inflate: {e}");
                return StatusCode::INTERNAL_SERVER_ERROR.into_response();
            }
            response
                .header(VARY, "accept-encoding")
                .body(Body::from(inflated))
        }
    };
    built.unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

/// The embedded file for a request path, under the name it is stored by. A path whose last segment has
/// no dot is a page of the app and gets index.html, as `MapFallbackToFile` did in 0.1.x. Nothing under
/// `/api` is the app's, so an unknown API path stays a 404.
fn find(path: &str) -> Option<(&str, Stored)> {
    let path = path.trim_start_matches('/');
    if path == "api" || path.starts_with("api/") {
        return None;
    }
    let name = if path.is_empty() { INDEX } else { path };
    if let Some(stored) = stored(name) {
        return Some((name, stored));
    }
    let last = path.rsplit('/').next().unwrap_or_default();
    if last.contains('.') {
        return None;
    }
    stored(INDEX).map(|stored| (INDEX, stored))
}

fn stored(name: &str) -> Option<Stored> {
    if let Some(file) = Gzipped::get(name) {
        return Some(Stored::Gzipped(file.data));
    }
    Raw::get(name).map(|file| Stored::Raw(file.data))
}

fn body(data: Cow<'static, [u8]>) -> Body {
    match data {
        Cow::Borrowed(bytes) => Body::from(Bytes::from_static(bytes)),
        Cow::Owned(bytes) => Body::from(bytes),
    }
}

fn cache_control(name: &str) -> &'static str {
    if name.starts_with("assets/") {
        HASHED
    } else {
        CHECK_EACH_TIME
    }
}

/// The types Vite writes. Anything else is plain bytes.
fn content_type(name: &str) -> &'static str {
    let extension = name.rsplit_once('.').map(|(_, e)| e).unwrap_or_default();
    match extension.to_ascii_lowercase().as_str() {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" | "map" => "application/json",
        "webmanifest" => "application/manifest+json",
        "txt" => "text/plain; charset=utf-8",
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
        "wasm" => "application/wasm",
        _ => "application/octet-stream",
    }
}

/// Whether Accept-Encoding allows gzip: named with a q above zero, or covered by `*`. No header means
/// identity, which is what clients without one expect, whatever RFC 9110 allows.
fn accepts_gzip(headers: &HeaderMap) -> bool {
    let mut gzip = None;
    let mut any = None;
    for value in headers.get_all(ACCEPT_ENCODING) {
        let Ok(text) = value.to_str() else { continue };
        for item in text.split(',') {
            let mut parts = item.split(';');
            let coding = parts.next().unwrap_or_default().trim();
            let quality = parts
                .find_map(|p| {
                    let p = p.trim();
                    p.strip_prefix("q=").or_else(|| p.strip_prefix("Q="))
                })
                .map_or(1.0, |q| q.trim().parse::<f32>().unwrap_or(0.0));
            if coding.eq_ignore_ascii_case("gzip") || coding.eq_ignore_ascii_case("x-gzip") {
                gzip = Some(quality > 0.0);
            } else if coding == "*" {
                any = Some(quality > 0.0);
            }
        }
    }
    gzip.or(any).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn accept(values: &[&str]) -> HeaderMap {
        let mut headers = HeaderMap::new();
        for value in values {
            headers.append(ACCEPT_ENCODING, HeaderValue::from_str(value).unwrap());
        }
        headers
    }

    #[test]
    fn accepts_gzip_reads_codings_and_quality() {
        assert!(accepts_gzip(&accept(&["gzip, deflate, br, zstd"])));
        assert!(accepts_gzip(&accept(&["br;q=1.0, GZIP;q=0.5"])));
        assert!(accepts_gzip(&accept(&["x-gzip"])));
        assert!(accepts_gzip(&accept(&["*"])));
        assert!(accepts_gzip(&accept(&["br", "gzip"])));
        assert!(!accepts_gzip(&accept(&[])));
        assert!(!accepts_gzip(&accept(&["identity"])));
        assert!(!accepts_gzip(&accept(&["br"])));
        assert!(!accepts_gzip(&accept(&["gzip;q=0"])));
        assert!(!accepts_gzip(&accept(&["gzip;q=0, *"])));
        assert!(!accepts_gzip(&accept(&["*;q=0"])));
        assert!(!accepts_gzip(&accept(&["gzip;q=nonsense"])));
    }

    #[test]
    fn cache_control_keeps_hashed_assets_for_good() {
        assert_eq!(cache_control("assets/index-Dvn-Ckvb.js"), HASHED);
        assert_eq!(cache_control("index.html"), CHECK_EACH_TIME);
        assert_eq!(cache_control("favicon.svg"), CHECK_EACH_TIME);
    }

    #[test]
    fn content_type_knows_what_vite_writes() {
        assert_eq!(content_type("index.html"), "text/html; charset=utf-8");
        assert_eq!(
            content_type("assets/index-1.js"),
            "text/javascript; charset=utf-8"
        );
        assert_eq!(
            content_type("assets/index-1.CSS"),
            "text/css; charset=utf-8"
        );
        assert_eq!(content_type("assets/a-1.woff2"), "font/woff2");
        assert_eq!(content_type("favicon.svg"), "image/svg+xml");
        assert_eq!(content_type("LICENSE"), "application/octet-stream");
    }

    #[test]
    fn find_gives_index_for_the_root_and_app_pages() {
        assert_eq!(find("/").map(|(name, _)| name), Some(INDEX));
        assert_eq!(find("/index.html").map(|(name, _)| name), Some(INDEX));
        assert_eq!(find("/devices").map(|(name, _)| name), Some(INDEX));
        assert_eq!(find("/server/clients").map(|(name, _)| name), Some(INDEX));
    }

    #[test]
    fn find_gives_nothing_for_missing_files_and_api_paths() {
        assert!(find("/assets/missing-1234.js").is_none());
        assert!(find("/missing.txt").is_none());
        assert!(find("/api").is_none());
        assert!(find("/api/unknown").is_none());
    }
}
