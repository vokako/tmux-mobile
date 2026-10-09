//! The web UI, served by the gateway on its own port (board #323).
//!
//! ONE source per process (`source()`): the `ui_dir` from config.toml when
//! set, else the `dist/` embedded at build time (feature `embed-ui`), else
//! none. A configured `ui_dir` that has no readable `index.html` is an
//! error the caller sees (`tmm ui` reports it) — never a silent fall back
//! to the embedded copy.
//!
//! Paths: the request target is percent-decoded once; `..`, `.`, an empty
//! segment, a backslash or a NUL refuses (404). A directory source resolves
//! the file with `canonicalize` and must stay under the canonical root, so
//! a symlink cannot reach outside it. The SPA fallback (index.html) answers
//! only NAVIGATIONS — `Accept` names `text/html` and the last segment has no
//! extension — so a missing script or stylesheet is a 404, not a page.
//!
//! index.html gets `<meta name="tmm-gateway" content="1">` injected into the
//! RESPONSE (the bytes on disk / in the binary are untouched): the page uses
//! it to default its connection address to its own origin. It carries no
//! token and no address. index.html and sw.js are `no-cache`; only the
//! content-hashed files under `/assets/` are `immutable`.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use tokio::io::{AsyncWrite, AsyncWriteExt};

#[cfg(feature = "embed-ui")]
mod embedded {
    include!(concat!(env!("OUT_DIR"), "/ui_embed.rs"));
}

pub const MARKER: &str = r#"<meta name="tmm-gateway" content="1">"#;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    Dir(PathBuf),
    Embedded,
    None,
}

static CONFIGURED: OnceLock<Option<String>> = OnceLock::new();

/// The gateway's `ui_dir` (call once, before the listener starts).
pub fn configure(ui_dir: Option<String>) {
    let _ = CONFIGURED.set(ui_dir);
}

/// The one source rule.
pub fn source_for(ui_dir: Option<&str>) -> Source {
    match ui_dir {
        Some(d) => Source::Dir(PathBuf::from(d)),
        None if cfg!(feature = "embed-ui") => Source::Embedded,
        None => Source::None,
    }
}

pub fn source() -> Source {
    source_for(CONFIGURED.get().and_then(|d| d.as_deref()))
}

/// Can `src` serve the UI? Ok(kind) or why not — the same resolution the
/// requests use, so `ui_info` never says ok for a directory that 404s.
pub fn check(src: &Source) -> Result<&'static str, String> {
    match src {
        Source::None => Err("this gateway has no web UI: it was built without embed-ui and no ui_dir is set".into()),
        Source::Embedded => lookup_embedded("index.html").map(|_| "embedded").ok_or_else(|| "the embedded UI has no index.html".into()),
        Source::Dir(d) => {
            if !d.is_absolute() {
                return Err(format!("ui_dir {} is not an absolute path", d.display()));
            }
            let root = d.canonicalize().map_err(|e| format!("ui_dir {} cannot be read ({})", d.display(), e.kind()))?;
            std::fs::File::open(root.join("index.html")).map(|_| "dir").map_err(|e| format!("ui_dir {} has no readable index.html ({})", d.display(), e.kind()))
        }
    }
}

#[cfg(feature = "embed-ui")]
fn lookup_embedded(rel: &str) -> Option<&'static [u8]> {
    embedded::FILES.binary_search_by(|(k, _)| (*k).cmp(rel)).ok().map(|i| embedded::FILES[i].1)
}
#[cfg(not(feature = "embed-ui"))]
fn lookup_embedded(_rel: &str) -> Option<&'static [u8]> {
    None
}

/// The request target as a relative file path: Ok("") = the root.
pub fn clean_path(target: &str) -> Option<String> {
    let path = target.split(['?', '#']).next().unwrap_or("");
    let path = path.strip_prefix('/')?;
    let decoded = urlencoding::decode(path).ok()?;
    if decoded.contains('\\') || decoded.contains('\0') {
        return None;
    }
    let segs: Vec<&str> = decoded.split('/').collect();
    for (i, s) in segs.iter().enumerate() {
        let last = i + 1 == segs.len();
        if *s == ".." || *s == "." || (s.is_empty() && !last) {
            return None;
        }
    }
    Some(decoded.trim_end_matches('/').to_string())
}

fn read(src: &Source, rel: &str) -> Option<Vec<u8>> {
    match src {
        Source::None => None,
        Source::Embedded => lookup_embedded(rel).map(<[u8]>::to_vec),
        Source::Dir(d) => {
            let root = d.canonicalize().ok()?;
            let full = root.join(rel).canonicalize().ok()?;
            if !full.starts_with(&root) || !full.is_file() {
                return None;
            }
            std::fs::read(full).ok()
        }
    }
}

fn is_navigation(rel: &str, accept: &str) -> bool {
    let last = rel.rsplit('/').next().unwrap_or("");
    accept.contains("text/html") && !last.contains('.')
}

pub fn content_type(rel: &str) -> &'static str {
    match Path::new(rel).extension().and_then(|e| e.to_str()).unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" | "map" => "application/json",
        "webmanifest" => "application/manifest+json",
        "png" => "image/png",
        "svg" => "image/svg+xml",
        "ico" => "image/x-icon",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        "ttf" => "font/ttf",
        "wasm" => "application/wasm",
        "txt" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

fn cache_control(rel: &str) -> &'static str {
    // Vite names every file under assets/ by its content hash.
    if rel.starts_with("assets/") { "public, max-age=31536000, immutable" } else { "no-cache" }
}

/// index.html with the hosted marker after `<head>` (or first, when none).
pub fn with_marker(html: &[u8]) -> Vec<u8> {
    let lower = String::from_utf8_lossy(html).to_ascii_lowercase();
    let at = lower.find("<head>").map(|i| i + "<head>".len()).unwrap_or(0);
    let mut out = Vec::with_capacity(html.len() + MARKER.len() + 1);
    out.extend_from_slice(&html[..at]);
    out.extend_from_slice(MARKER.as_bytes());
    out.extend_from_slice(&html[at..]);
    out
}

/// The response for one static request: (status, content type, cache, body).
pub fn respond(src: &Source, target: &str, accept: &str) -> (u16, &'static str, &'static str, Vec<u8>) {
    let not_found = |why: &str| (404, "text/plain; charset=utf-8", "no-cache", format!("{why}\n").into_bytes());
    if let Err(why) = check(src) {
        return not_found(&why);
    }
    let Some(rel) = clean_path(target) else { return not_found("not found") };
    let rel = if rel.is_empty() { "index.html".to_string() } else { rel };
    let (rel, body) = match read(src, &rel) {
        Some(b) => (rel, b),
        None if is_navigation(&rel, accept) => match read(src, "index.html") {
            Some(b) => ("index.html".to_string(), b),
            None => return not_found("not found"),
        },
        None => return not_found("not found"),
    };
    let body = if rel == "index.html" { with_marker(&body) } else { body };
    (200, content_type(&rel), cache_control(&rel), body)
}

/// Write the response; HEAD gets the same headers and no body.
pub async fn serve<S: AsyncWrite + Unpin>(mut stream: S, head: bool, target: &str, accept: &str) {
    let (status, ctype, cache, body) = respond(&source(), target, accept);
    let reason = if status == 200 { "OK" } else { "Not Found" };
    let header = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nCache-Control: {cache}\r\nX-Content-Type-Options: nosniff\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(header.as_bytes()).await;
    if !head {
        let _ = stream.write_all(&body).await;
    }
    let _ = stream.flush().await;
    let _ = stream.shutdown().await;
}

/// 400 for a request this port does not speak.
pub async fn bad_request<S: AsyncWrite + Unpin>(mut stream: S) {
    let _ = stream.write_all(b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await;
    let _ = stream.shutdown().await;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn site(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("ui-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("assets")).unwrap();
        std::fs::write(d.join("index.html"), "<!doctype html><html><head><title>t</title></head></html>").unwrap();
        std::fs::write(d.join("assets/app-abc123.js"), "console.log(1)").unwrap();
        std::fs::write(d.join("sw.js"), "self").unwrap();
        d
    }

    #[test]
    fn paths_are_decoded_once_and_never_leave_the_root() {
        assert_eq!(clean_path("/"), Some("".into()));
        assert_eq!(clean_path("/assets/a.js?v=1"), Some("assets/a.js".into()));
        assert_eq!(clean_path("/a%20b.png"), Some("a b.png".into()));
        for bad in ["/../etc/passwd", "/%2e%2e/etc/passwd", "/assets/../../x", "/a//b", "/a\\b", "/%5c..%5cx", "/a%00b", "/./x", "relative"] {
            assert_eq!(clean_path(bad), None, "{bad}");
        }
    }

    #[test]
    fn a_directory_source_serves_files_with_types_and_cache_rules() {
        let d = site("serve");
        let src = Source::Dir(d.clone());
        let (s, t, c, b) = respond(&src, "/", "text/html");
        assert_eq!((s, t, c), (200, "text/html; charset=utf-8", "no-cache"));
        let html = String::from_utf8(b).unwrap();
        assert!(html.starts_with("<!doctype html><html><head><meta name=\"tmm-gateway\" content=\"1\"><title>"), "{html}");
        assert!(!std::fs::read_to_string(d.join("index.html")).unwrap().contains("tmm-gateway"), "the file on disk is untouched");
        assert_eq!(respond(&src, "/assets/app-abc123.js", "*/*").1, "text/javascript; charset=utf-8");
        assert_eq!(respond(&src, "/assets/app-abc123.js", "*/*").2, "public, max-age=31536000, immutable");
        assert_eq!(respond(&src, "/sw.js", "*/*").2, "no-cache", "the service worker is never cached long");
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn only_navigations_fall_back_to_index_and_missing_assets_404() {
        let d = site("spa");
        let src = Source::Dir(d.clone());
        let (s, _, _, b) = respond(&src, "/projects/test", "text/html,application/xhtml+xml");
        assert_eq!(s, 200);
        assert!(String::from_utf8(b).unwrap().contains("tmm-gateway"), "a navigation gets the page");
        assert_eq!(respond(&src, "/assets/missing-xyz.js", "*/*").0, 404, "a missing script is not a page");
        assert_eq!(respond(&src, "/assets/missing.css", "text/html").0, 404, "an extension is never a navigation");
        assert_eq!(respond(&src, "/projects/test", "*/*").0, 404, "not a navigation without text/html");
        std::fs::remove_dir_all(&d).ok();
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_cannot_reach_outside_the_root() {
        let d = site("link");
        let outside = d.with_extension("secret");
        std::fs::write(&outside, "SECRET").unwrap();
        std::os::unix::fs::symlink(&outside, d.join("leak.txt")).unwrap();
        std::os::unix::fs::symlink("/etc", d.join("etc")).unwrap();
        let src = Source::Dir(d.clone());
        assert_eq!(respond(&src, "/leak.txt", "*/*").0, 404);
        assert_eq!(respond(&src, "/etc/hostname", "*/*").0, 404);
        std::fs::remove_dir_all(&d).ok();
        std::fs::remove_file(&outside).ok();
    }

    #[test]
    fn the_source_rule_and_its_check() {
        assert_eq!(source_for(Some("/x")), Source::Dir("/x".into()));
        assert_eq!(source_for(None), if cfg!(feature = "embed-ui") { Source::Embedded } else { Source::None });
        assert!(check(&Source::None).unwrap_err().contains("no web UI"));
        assert!(check(&Source::Dir("rel/dist".into())).unwrap_err().contains("not an absolute path"));
        assert!(check(&Source::Dir("/nonexistent/dist".into())).unwrap_err().contains("cannot be read"));
        let d = site("check");
        assert_eq!(check(&Source::Dir(d.clone())), Ok("dir"));
        std::fs::remove_file(d.join("index.html")).unwrap();
        assert!(check(&Source::Dir(d.clone())).unwrap_err().contains("no readable index.html"), "a dir without index is not ok");
        assert_eq!(respond(&Source::Dir(d.clone()), "/assets/app-abc123.js", "*/*").0, 404, "and serves nothing");
        std::fs::remove_dir_all(&d).ok();
    }
}
