//! The token-signed HTTP download side-channel on the WS port: same TCP
//! listener, requests that *look like* GET /dl are peeled off before the
//! WebSocket upgrade and streamed with Range support. Split from server.rs
//! 2026-07-22 — content unchanged.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;

use hmac::{Hmac, Mac};
use sha2::Sha256;

/// A download URL lives one minute: the client fetches it at once (and
/// re-signs on every resume), so a leaked URL is dead before it travels.
const DL_TOKEN_TTL_SECS: u64 = 60;
/// A STREAM URL lives as long as a viewing does (board #182). A `<video>`
/// element keeps coming back to the same URL for the whole playback —
/// Chromium suspends loading once its buffer is full and re-issues a Range
/// request from the next offset, every seek is a fresh request — so a 60 s
/// signature 403s the second request a minute into the film. The lifetime is
/// carried IN the signature as an absolute expiry (`exp`), so the verifier
/// needs no second rule; the request kind picks the lifetime at mint time.
/// 4 h is the ceiling: a leaked stream URL replays for at most that long,
/// which covers any film and nothing more; downloads keep their minute.
pub(super) const DL_STREAM_TTL_SECS: u64 = 4 * 3600;

fn unix_now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs()
}

/// When a URL minted now expires: a download in a minute, a stream in hours.
pub(super) fn download_expiry(stream: bool) -> u64 {
    unix_now() + if stream { DL_STREAM_TTL_SECS } else { DL_TOKEN_TTL_SECS }
}

/// Only what a `<video>` plays may carry a stream signature — the long life
/// is for media elements, never for arbitrary files (`stream: true` on a
/// .env is refused at mint AND at /dl).
pub(super) fn streams(name: &str) -> bool {
    media_content_type(name).starts_with("video/")
}

/// The signed tuple. The MODE is a literal in the MAC input, so a stream
/// signature and a download signature over the same path and expiry are
/// different strings: a long-lived one can only ever verify as a stream.
fn mac_input(stream: bool, path: &str, exp: u64) -> String {
    if stream { format!("dl:stream:{}:{}", path, exp) } else { format!("dl:{}:{}", path, exp) }
}

pub(super) fn sign_download(token: &str, stream: bool, path: &str, exp: u64) -> String {
    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(token.as_bytes()).unwrap();
    mac.update(mac_input(stream, path, exp).as_bytes());
    hex::encode(mac.finalize().into_bytes())
}

/// The signature is verified by the MAC itself (`verify_slice` is constant
/// time), never by comparing hex strings with `==`, which would return at the
/// first wrong nibble and time-leak the expected signature byte by byte.
/// `exp` and the mode are bound into the MAC, so a client cannot extend a
/// URL's life or relabel a download as a stream; each mode has its own
/// ceiling, and a stream signature verifies only for a path that streams.
fn verify_download(token: &str, stream: bool, path: &str, exp: u64, sig: &str) -> bool {
    let now = unix_now();
    let ttl = if stream { DL_STREAM_TTL_SECS } else { DL_TOKEN_TTL_SECS };
    if exp < now || exp > now + ttl { return false; }
    if stream && !streams(path) { return false; }
    let Ok(sig_bytes) = hex::decode(sig) else { return false };
    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(token.as_bytes()).unwrap();
    mac.update(mac_input(stream, path, exp).as_bytes());
    mac.verify_slice(&sig_bytes).is_ok()
}

// WebSocket frame / message limits. A legitimate `fs_upload` can carry a
// file up to fs::MAX_READ_SIZE (50 MB) inside a base64 string (~67 MB text),
// plus JSON envelope + encryption overhead. 80 MB accommodates that with
// margin; anything bigger is almost certainly malformed or abusive.
// tokio-tungstenite's default (64 MB) is too small for a max-size upload,
// and without an explicit cap an attacker could force per-connection buffer
// growth up to that limit on every frame.

/// Find the end of the HTTP header block (offset of the CRLFCRLF terminator).
fn find_header_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n")
}

/// The byte window a `Range` header asks for, inclusive and clamped to the
/// file, or `None` for no/unsupported Range (a server may ignore Range and
/// answer 200 with the full body). `Err` = unsatisfiable (start past the end).
///
/// `bytes=N-` is what our resume client sends. `bytes=N-M` is what a media
/// element sends (board #182): WebKit probes with `bytes=0-1` and refuses to
/// play unless the answer is a 206 of exactly two bytes; Chromium seeks with
/// bounded ranges too. Suffix (`-N`) and multi-range forms stay unsupported.
fn parse_range(req: &str, size: u64) -> Result<Option<(u64, u64)>, ()> {
    for line in req.lines() {
        let Some((k, v)) = line.split_once(':') else { continue };
        if !k.trim().eq_ignore_ascii_case("range") {
            continue;
        }
        let Some(spec) = v.trim().strip_prefix("bytes=") else { return Ok(None) };
        let Some((start, rest)) = spec.split_once('-') else { return Ok(None) };
        let Ok(start) = start.trim().parse::<u64>() else { return Ok(None) };
        let end = if rest.trim().is_empty() {
            size.saturating_sub(1)
        } else {
            let Ok(end) = rest.trim().parse::<u64>() else { return Ok(None) };
            end.min(size.saturating_sub(1))
        };
        if size == 0 || start >= size || start > end { return Err(()) }
        return Ok(Some((start, end)));
    }
    Ok(None)
}

/// Decide whether a peeked request prelude is an HTTP /dl download rather
/// than a WebSocket upgrade. Both arrive as HTTP GET; we look for the
/// "/dl?" path segment in the request LINE only, tolerating a reverse-proxy
/// path prefix (e.g. "GET /tmux/dl?path=..." when the proxy doesn't strip
/// its location prefix).
pub(super) fn looks_like_dl_request(prelude: &[u8]) -> bool {
    if !prelude.starts_with(b"GET ") {
        return false;
    }
    let line_end = prelude
        .iter()
        .position(|&b| b == b'\r' || b == b'\n')
        .unwrap_or(prelude.len());
    prelude[..line_end].windows(4).any(|w| w == b"/dl?")
}

pub(super) async fn handle_http_download<S>(mut stream: S, addr: SocketAddr, token: Arc<String>)
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send,
{
    use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};

    // Read until the full header block has arrived. Reverse proxies often
    // deliver the request line and headers across multiple TCP segments;
    // a single read() truncates the query string mid-signature and rejects
    // a perfectly valid request with 403. (Direct LAN/Tailscale clients
    // virtually always deliver everything in one segment, which is why
    // this only ever bit through a proxy.)
    let mut buf: Vec<u8> = Vec::with_capacity(4096);
    let mut tmp = [0u8; 2048];
    let header_end = loop {
        if let Some(pos) = find_header_end(&buf) {
            break pos;
        }
        if buf.len() > 16 * 1024 {
            return; // oversized header block — not a legitimate /dl request
        }
        match stream.read(&mut tmp).await {
            Ok(0) => return,
            Ok(n) => buf.extend_from_slice(&tmp[..n]),
            Err(_) => return,
        }
    };
    let req = String::from_utf8_lossy(&buf[..header_end]).to_string();
    let first_line = req.lines().next().unwrap_or("");

    // Parse "GET <path>/dl?path=...&ts=...&sig=... HTTP/1.1". Locate the
    // "/dl?" segment instead of assuming it starts the path, so a proxy
    // prefix doesn't break query extraction.
    let url_part = first_line.split_whitespace().nth(1).unwrap_or("");
    let query = match url_part.find("/dl?") {
        Some(i) => &url_part[i + 4..],
        None => "",
    };
    let params: HashMap<&str, &str> = query.split('&')
        .filter_map(|p| p.split_once('='))
        .collect();

    let path = match params.get("path") {
        Some(p) => urlencoding::decode(p).unwrap_or_default().to_string(),
        None => { let _ = stream.write_all(b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\n\r\n").await; return; }
    };
    let exp: u64 = params.get("exp").and_then(|s| s.parse().ok()).unwrap_or(0);
    let sig = params.get("sig").unwrap_or(&"");
    let as_stream = params.get("stream").is_some_and(|v| *v == "1");

    if !verify_download(&token, as_stream, &path, exp, sig) {
        eprintln!("🚫 HTTP download rejected for {} (invalid sig)", addr);
        let _ = stream.write_all(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\n\r\n").await;
        let _ = stream.flush().await;
        return;
    }

    // Read file and stream response
    let file_path = std::path::Path::new(&path);
    let metadata = match std::fs::metadata(file_path) {
        Ok(m) => m,
        Err(_) => {
            let _ = stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n").await;
            let _ = stream.flush().await;
            return;
        }
    };
    let name = file_path.file_name().and_then(|n| n.to_str()).unwrap_or("file");
    let size = metadata.len();

    // Range support: lets the client RESUME an interrupted transfer instead
    // of restarting from byte 0. Critical through reverse proxies on the
    // public internet, where long-lived large responses get cut by proxy
    // idle/total timeouts — without resume, a 100 MB file that dies at 95%
    // restarts from scratch and may never complete.
    // `Connection: close` matters behind reverse proxies: we serve one
    // request per TCP connection and then drop it. Without the header,
    // HTTP/1.1 defaults to keep-alive and the proxy may pool the (already
    // closed) backend connection, surfacing as intermittent 502s on the
    // next download.
    let range = match parse_range(&req, size) {
        Ok(r) => r,
        Err(()) => {
            let _ = stream.write_all(format!("HTTP/1.1 416 Range Not Satisfiable\r\nContent-Range: bytes */{}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n", size).as_bytes()).await;
            let _ = stream.flush().await;
            return;
        }
    };
    // Images and videos declare their REAL type: a chat `<img>` and a Files
    // `<video>` stream through /dl, and SVG never renders from
    // `application/octet-stream` (browsers do not sniff SVG — the dashed
    // fallback frame in every bubble, 2026-09-08). `Content-Disposition:
    // attachment` stays on everything, so a top-level navigation still
    // downloads instead of rendering — an SVG's scripts never execute from an
    // <img> context, and never get a document here. Everything else keeps
    // octet-stream: downloads, not documents.
    let ctype = media_content_type(&name);
    let header = match range {
        Some((start, end)) => format!(
            "HTTP/1.1 206 Partial Content\r\nContent-Type: {}\r\nContent-Disposition: attachment; filename=\"{}\"\r\nContent-Length: {}\r\nContent-Range: bytes {}-{}/{}\r\nAccept-Ranges: bytes\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Expose-Headers: Content-Length, Content-Range, Accept-Ranges\r\nConnection: close\r\n\r\n",
            ctype, name, end - start + 1, start, end, size
        ),
        None => format!(
            "HTTP/1.1 200 OK\r\nContent-Type: {}\r\nContent-Disposition: attachment; filename=\"{}\"\r\nContent-Length: {}\r\nAccept-Ranges: bytes\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Expose-Headers: Content-Length, Content-Range, Accept-Ranges\r\nConnection: close\r\n\r\n",
            ctype, name, size
        ),
    };
    if stream.write_all(header.as_bytes()).await.is_err() { return; }

    // Stream file in chunks
    let mut file = match tokio::fs::File::open(&path).await {
        Ok(f) => f,
        Err(_) => return,
    };
    let mut remaining = size;
    if let Some((start, end)) = range {
        if file.seek(std::io::SeekFrom::Start(start)).await.is_err() {
            return;
        }
        remaining = end - start + 1;
    }
    let mut file = file.take(remaining);
    let mut chunk = vec![0u8; 65536];
    loop {
        let n = match file.read(&mut chunk).await {
            Ok(0) => break,
            Ok(n) => n,
            Err(_) => break,
        };
        if stream.write_all(&chunk[..n]).await.is_err() { break; }
    }
    // Flush any data still sitting in BufStream's write buffer — on drop
    // that buffer is discarded and the tail of the file would be lost.
    let _ = stream.flush().await;
}

/// The Content-Type for /dl: image and video extensions get their real type
/// so a chat `<img>` and a Files `<video>` can render them (SVG hard-requires
/// `image/svg+xml`; bitmaps are sniffed but say the truth anyway; WebKit
/// wants a `video/*` type before it asks AVFoundation). Media never execute
/// anything. Everything else stays octet-stream — /dl serves DOWNLOADS, not
/// documents (html/js/css must never render off this endpoint: same-origin +
/// a token-signed URL would be an XSS gift). Same table as fs::mime_hint's
/// video rows: what Files calls a video, /dl serves as one.
fn media_content_type(name: &str) -> &'static str {
    let ext = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "gif" => "image/gif",
        "bmp" => "image/bmp",
        "svg" => "image/svg+xml",
        "mp4" | "m4v" => "video/mp4",
        "webm" => "video/webm",
        "mov" => "video/quicktime",
        "mkv" => "video/x-matroska",
        "ogv" => "video/ogg",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_declare_their_real_type_everything_else_stays_a_download() {
        // SVG is the load-bearing case: an <img> never renders it from
        // octet-stream (the dashed fallback in every chat bubble, 2026-09-08).
        assert_eq!(media_content_type("harness-layers.svg"), "image/svg+xml");
        assert_eq!(media_content_type("SHOT.PNG"), "image/png");
        assert_eq!(media_content_type("a.b.jpeg"), "image/jpeg");
        // Video (board #182): a Files <video> streams off /dl; WebKit wants the type.
        assert_eq!(media_content_type("demo.MP4"), "video/mp4");
        assert_eq!(media_content_type("clip.m4v"), "video/mp4");
        assert_eq!(media_content_type("screen.webm"), "video/webm");
        assert_eq!(media_content_type("take.mov"), "video/quicktime");
        assert_eq!(media_content_type("rip.mkv"), "video/x-matroska");
        // Documents and executables must never render off /dl.
        assert_eq!(media_content_type("index.html"), "application/octet-stream");
        assert_eq!(media_content_type("report.pdf"), "application/octet-stream");
        assert_eq!(media_content_type("player.js"), "application/octet-stream");
        assert_eq!(media_content_type("noext"), "application/octet-stream");
    }

    #[test]
    fn range_header_open_ended_parses() {
        let req = "GET /dl?path=x HTTP/1.1\r\nHost: h\r\nRange: bytes=12345-\r\n";
        assert_eq!(parse_range(req, 20_000), Ok(Some((12345, 19_999))));
        // From byte 0 is a valid range too: a media element's first request.
        assert_eq!(parse_range("Range: bytes=0-\r\n", 10), Ok(Some((0, 9))));
    }

    #[test]
    fn range_header_case_insensitive() {
        let req = "GET /dl?path=x HTTP/1.1\r\nrange: bytes=7-\r\n";
        assert_eq!(parse_range(req, 100), Ok(Some((7, 99))));
    }

    #[test]
    fn range_header_bounded_form_serves_exactly_that_window() {
        // Board #182: WebKit probes a video with `bytes=0-1` and needs 206 of
        // exactly two bytes; a seek asks for a bounded window. Clamp to the file.
        assert_eq!(parse_range("Range: bytes=0-1\r\n", 1_000), Ok(Some((0, 1))));
        assert_eq!(parse_range("Range: bytes=500-999\r\n", 1_000), Ok(Some((500, 999))));
        assert_eq!(parse_range("Range: bytes=500-5000\r\n", 1_000), Ok(Some((500, 999))), "end clamps to the last byte");
        // Unsatisfiable: past the end, inverted, or an empty file → 416.
        assert_eq!(parse_range("Range: bytes=1000-\r\n", 1_000), Err(()));
        assert_eq!(parse_range("Range: bytes=9-3\r\n", 1_000), Err(()));
        assert_eq!(parse_range("Range: bytes=0-\r\n", 0), Err(()));
    }

    #[test]
    fn range_header_unsupported_forms_fall_back_to_the_full_body() {
        // Suffix and malformed forms are not emitted by our clients; a server
        // may ignore Range and answer 200 (RFC 7233).
        assert_eq!(parse_range("Range: bytes=-500\r\n", 1_000), Ok(None));
        assert_eq!(parse_range("Range: items=0-5\r\n", 1_000), Ok(None));
        assert_eq!(parse_range("Range: bytes=abc-\r\n", 1_000), Ok(None));
        assert_eq!(parse_range("GET / HTTP/1.1\r\nHost: h\r\n", 1_000), Ok(None));
    }

    #[test]
    fn dl_detection_with_and_without_proxy_prefix() {
        assert!(looks_like_dl_request(b"GET /dl?path=a&ts=1&sig=b HTTP/1.1\r\n"));
        // Reverse proxy that forwards its location prefix unstripped.
        assert!(looks_like_dl_request(b"GET /tmux/dl?path=a HTTP/1.1\r\n"));
        assert!(!looks_like_dl_request(b"GET / HTTP/1.1\r\nUpgrade: websocket\r\n"));
        // "/dl?" appearing only in a header (not the request line) must not match.
        assert!(!looks_like_dl_request(b"GET /ws HTTP/1.1\r\nReferer: /dl?x\r\n"));
        assert!(!looks_like_dl_request(b"POST /dl?path=a HTTP/1.1\r\n"));
    }

    #[test]
    fn header_end_detection() {
        assert_eq!(find_header_end(b"GET / HTTP/1.1\r\nHost: h\r\n\r\nbody"), Some(23));
        assert_eq!(find_header_end(b"GET / HTTP/1.1\r\nHost: h\r\n"), None);
    }

    #[test]
    fn download_signature_verifies_only_the_exact_mac() {
        let exp = download_expiry(false);
        let sig = sign_download("tok", false, "/a/b.txt", exp);
        assert!(verify_download("tok", false, "/a/b.txt", exp, &sig));
        // Any change to the signed tuple, or to the signature, is rejected.
        assert!(!verify_download("tok", false, "/a/c.txt", exp, &sig), "path is bound");
        assert!(!verify_download("tok", false, "/a/b.txt", exp + 1, &sig), "exp is bound: a client cannot extend a URL's life");
        assert!(!verify_download("other", false, "/a/b.txt", exp, &sig), "token is bound");
        let mut flipped = sig.clone();
        flipped.replace_range(0..1, if sig.starts_with('0') { "1" } else { "0" });
        assert!(!verify_download("tok", false, "/a/b.txt", exp, &flipped));
        // Not hex, wrong length, empty: rejected without panicking.
        assert!(!verify_download("tok", false, "/a/b.txt", exp, "zz"));
        assert!(!verify_download("tok", false, "/a/b.txt", exp, &sig[..10]));
        assert!(!verify_download("tok", false, "/a/b.txt", exp, ""));
        // Expired.
        let gone = unix_now() - 1;
        assert!(!verify_download("tok", false, "/a/b.txt", gone, &sign_download("tok", false, "/a/b.txt", gone)));
        // A download signature never outlives its minute, however it is labelled.
        let long = unix_now() + DL_TOKEN_TTL_SECS + 5;
        assert!(!verify_download("tok", false, "/a/b.txt", long, &sign_download("tok", false, "/a/b.txt", long)));
    }

    #[test]
    fn stream_signatures_are_a_separate_mode_for_video_only() {
        // Board #182: a <video> re-requests its URL for the whole playback,
        // so a stream signature lives hours — and therefore ONLY for media.
        let now = unix_now();
        let dl = download_expiry(false);
        let st = download_expiry(true);
        assert!(dl >= now + DL_TOKEN_TTL_SECS && dl <= now + DL_TOKEN_TTL_SECS + 1);
        assert!(st >= now + DL_STREAM_TTL_SECS && st <= now + DL_STREAM_TTL_SECS + 1);
        assert_eq!(DL_STREAM_TTL_SECS, 4 * 3600, "a leaked URL replays for at most one film");
        let sig = sign_download("tok", true, "/v.mp4", st);
        assert!(verify_download("tok", true, "/v.mp4", st, &sig));
        // The mode is in the MAC: the same tuple signed as a download is not a stream, and vice versa.
        assert_ne!(sig, sign_download("tok", false, "/v.mp4", st));
        assert!(!verify_download("tok", false, "/v.mp4", st, &sig), "a stream signature does not verify as a download");
        assert!(!verify_download("tok", true, "/v.mp4", dl, &sign_download("tok", false, "/v.mp4", dl)), "a download signature does not verify as a stream");
        // A stream signature over a non-video path is refused even when the MAC is right.
        let env = sign_download("tok", true, "/srv/.env", st);
        assert!(!streams("/srv/.env"));
        assert!(!verify_download("tok", true, "/srv/.env", st, &env), "the long life is for media elements only");
        assert!(!verify_download("tok", true, "/srv/index.html", st, &sign_download("tok", true, "/srv/index.html", st)));
        assert!(!verify_download("tok", true, "/shot.png", st, &sign_download("tok", true, "/shot.png", st)), "images arrive as bytes; they do not stream");
        // Even a correctly signed expiry beyond the ceiling is refused.
        let far = now + DL_STREAM_TTL_SECS + 60;
        assert!(!verify_download("tok", true, "/v.mp4", far, &sign_download("tok", true, "/v.mp4", far)));
    }
}
