//! The token-signed HTTP download side-channel on the WS port: same TCP
//! listener, requests that *look like* GET /dl are peeled off before the
//! WebSocket upgrade and streamed with Range support. Split from server.rs
//! 2026-07-22 — content unchanged.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;

use hmac::{Hmac, Mac};
use sha2::Sha256;

const DL_TOKEN_TTL_SECS: u64 = 60;

pub(super) fn sign_download(token: &str, path: &str, ts: u64) -> String {
    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(token.as_bytes()).unwrap();
    mac.update(format!("dl:{}:{}", path, ts).as_bytes());
    hex::encode(mac.finalize().into_bytes())
}

/// The signature is verified by the MAC itself (`verify_slice` is constant
/// time), never by comparing hex strings with `==`, which would return at the
/// first wrong nibble and time-leak the expected signature byte by byte.
fn verify_download(token: &str, path: &str, ts: u64, sig: &str) -> bool {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
    if now.saturating_sub(ts) > DL_TOKEN_TTL_SECS { return false; }
    let Ok(sig_bytes) = hex::decode(sig) else { return false };
    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(token.as_bytes()).unwrap();
    mac.update(format!("dl:{}:{}", path, ts).as_bytes());
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

/// Parse a `Range: bytes=N-` request header out of a raw header block.
/// Only the open-ended single-range form is supported — that's the only
/// form our resume client emits. Any other form is ignored (a server is
/// allowed to ignore Range and answer 200 with the full body).
fn parse_range_start(req: &str) -> Option<u64> {
    for line in req.lines() {
        let Some((k, v)) = line.split_once(':') else { continue };
        if !k.trim().eq_ignore_ascii_case("range") {
            continue;
        }
        let spec = v.trim().strip_prefix("bytes=")?;
        let (start, rest) = spec.split_once('-')?;
        if !rest.is_empty() {
            return None; // "N-M" / "-N" suffix form: ignore, serve full body
        }
        return start.parse().ok();
    }
    None
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
    let ts: u64 = params.get("ts").and_then(|s| s.parse().ok()).unwrap_or(0);
    let sig = params.get("sig").unwrap_or(&"");

    if !verify_download(&token, &path, ts, sig) {
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
    let range_start = parse_range_start(&req).filter(|&s| s > 0 && s < size);
    let header = match range_start {
        Some(start) => format!(
            "HTTP/1.1 206 Partial Content\r\nContent-Type: application/octet-stream\r\nContent-Disposition: attachment; filename=\"{}\"\r\nContent-Length: {}\r\nContent-Range: bytes {}-{}/{}\r\nAccept-Ranges: bytes\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Expose-Headers: Content-Length, Content-Range, Accept-Ranges\r\nConnection: close\r\n\r\n",
            name, size - start, start, size - 1, size
        ),
        None => format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Disposition: attachment; filename=\"{}\"\r\nContent-Length: {}\r\nAccept-Ranges: bytes\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Expose-Headers: Content-Length, Content-Range, Accept-Ranges\r\nConnection: close\r\n\r\n",
            name, size
        ),
    };
    if stream.write_all(header.as_bytes()).await.is_err() { return; }

    // Stream file in chunks
    let mut file = match tokio::fs::File::open(&path).await {
        Ok(f) => f,
        Err(_) => return,
    };
    if let Some(start) = range_start {
        if file.seek(std::io::SeekFrom::Start(start)).await.is_err() {
            return;
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn range_header_open_ended_parses() {
        let req = "GET /dl?path=x HTTP/1.1\r\nHost: h\r\nRange: bytes=12345-\r\n";
        assert_eq!(parse_range_start(req), Some(12345));
    }

    #[test]
    fn range_header_case_insensitive() {
        let req = "GET /dl?path=x HTTP/1.1\r\nrange: bytes=7-\r\n";
        assert_eq!(parse_range_start(req), Some(7));
    }

    #[test]
    fn range_header_bounded_form_ignored() {
        // "N-M" and suffix forms are not emitted by our client; server
        // falls back to a full 200 response.
        assert_eq!(parse_range_start("Range: bytes=0-499\r\n"), None);
        assert_eq!(parse_range_start("Range: bytes=-500\r\n"), None);
        assert_eq!(parse_range_start("GET / HTTP/1.1\r\nHost: h\r\n"), None);
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
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
        let sig = sign_download("tok", "/a/b.txt", now);
        assert!(verify_download("tok", "/a/b.txt", now, &sig));
        // Any change to the signed tuple, or to the signature, is rejected.
        assert!(!verify_download("tok", "/a/c.txt", now, &sig), "path is bound");
        assert!(!verify_download("tok", "/a/b.txt", now - 1, &sig), "ts is bound");
        assert!(!verify_download("other", "/a/b.txt", now, &sig), "token is bound");
        let mut flipped = sig.clone();
        flipped.replace_range(0..1, if sig.starts_with('0') { "1" } else { "0" });
        assert!(!verify_download("tok", "/a/b.txt", now, &flipped));
        // Not hex, wrong length, empty: rejected without panicking.
        assert!(!verify_download("tok", "/a/b.txt", now, "zz"));
        assert!(!verify_download("tok", "/a/b.txt", now, &sig[..10]));
        assert!(!verify_download("tok", "/a/b.txt", now, ""));
        // Expired.
        assert!(!verify_download("tok", "/a/b.txt", now - DL_TOKEN_TTL_SECS - 1, &sign_download("tok", "/a/b.txt", now - DL_TOKEN_TTL_SECS - 1)));
    }
}
