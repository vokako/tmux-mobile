//! Is OUR gateway already answering on this machine? (board #323)
//!
//! A real loopback login with this machine's own token, compared on machine
//! id — the server's existing auth, so no unauthenticated method is added.
//! Three verdicts, and only `None` (the connection was refused: nothing is
//! listening) lets a caller start a server:
//!
//! - `Ours` — the login succeeded and the machine id is ours.
//! - `Occupied` — something answers but cannot be proven ours: not a
//!   WebSocket, the token is refused, another machine's id, a TLS failure
//!   (including a certificate that does not name the loopback address), or
//!   no answer within the deadline. Never treated as free.
//! - `None` — connection refused.
//!
//! The address and scheme come from the effective `Config`: a wildcard bind
//! (`0.0.0.0` / `::`) is reached on the matching loopback, a specific address
//! on itself; `wss` when a TLS certificate is configured, trusting exactly
//! that certificate (a self-signed gateway proves itself) and checking it
//! against the address dialled.

use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;

use crate::config::Config;

pub const DEADLINE: Duration = Duration::from_millis(1500);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    Ours { machine_id: String, url: String },
    Occupied(String),
    None,
}

/// Where a local client reaches the gateway `cfg` describes.
pub fn local_url(cfg: &Config) -> String {
    let host = match cfg.host.trim() {
        "" | "0.0.0.0" => "127.0.0.1".to_string(),
        "::" | "[::]" => "[::1]".to_string(),
        h if h.contains(':') && !h.starts_with('[') => format!("[{h}]"),
        h => h.to_string(),
    };
    let scheme = if cfg.tls_cert.is_some() && cfg.tls_key.is_some() { "wss" } else { "ws" };
    format!("{scheme}://{host}:{}", cfg.port)
}

/// The pure verdict over what the login answered (tested without a socket).
pub(crate) fn judge(answer: &serde_json::Value, our_machine: &str, url: &str) -> Verdict {
    if let Some(err) = answer.get("error") {
        let msg = err.get("message").and_then(|m| m.as_str()).unwrap_or("auth refused");
        return Verdict::Occupied(format!("a gateway answers at {url} but refused this machine's token ({msg})"));
    }
    let r = answer.get("result");
    if r.and_then(|r| r.get("authenticated")).and_then(|v| v.as_bool()) != Some(true) {
        return Verdict::Occupied(format!("{url} answered, but not as a tmux-mobile gateway ({})", excerpt(answer)));
    }
    let id = r.and_then(|r| r.get("machine_id")).and_then(|v| v.as_str()).unwrap_or("");
    if id == our_machine && !id.is_empty() {
        Verdict::Ours { machine_id: id.to_string(), url: url.to_string() }
    } else {
        Verdict::Occupied(format!("{url} is another machine's gateway (machine id {id})"))
    }
}

fn excerpt(v: &serde_json::Value) -> String {
    let t = v.to_string();
    t.chars().take(120).collect()
}

pub async fn probe(cfg: &Config) -> Verdict {
    let url = local_url(cfg);
    match tokio::time::timeout(DEADLINE, attempt(cfg, &url)).await {
        Ok(v) => v,
        Err(_) => Verdict::Occupied(format!("{url} did not answer within {} ms", DEADLINE.as_millis())),
    }
}

async fn attempt(cfg: &Config, url: &str) -> Verdict {
    let authority = url.split("://").nth(1).unwrap_or_default().to_string();
    let tcp = match TcpStream::connect(&authority.replace(['[', ']'], "")).await {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::ConnectionRefused => return Verdict::None,
        Err(e) => return Verdict::Occupied(format!("cannot reach {url}: {e}")),
    };
    let answer = if url.starts_with("wss://") {
        let tls = match tls_connect(cfg, &authority, tcp).await {
            Ok(s) => s,
            Err(e) => return Verdict::Occupied(format!("TLS to {url} failed: {e}")),
        };
        login(tls, url, &cfg.token).await
    } else {
        login(tcp, url, &cfg.token).await
    };
    match answer {
        Ok(v) => judge(&v, &cfg.machine_id, url),
        Err(e) => Verdict::Occupied(format!("{url} is not a tmux-mobile gateway ({e})")),
    }
}

async fn login<S>(stream: S, url: &str, token: &str) -> Result<serde_json::Value, String>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let (mut ws, _) = tokio_tungstenite::client_async(url, stream).await.map_err(|e| e.to_string())?;
    let hello = serde_json::json!({ "id": 1, "method": "auth", "params": { "token": token } });
    ws.send(Message::Text(hello.to_string().into())).await.map_err(|e| e.to_string())?;
    while let Some(m) = ws.next().await {
        match m.map_err(|e| e.to_string())? {
            Message::Text(t) => {
                let v: serde_json::Value = serde_json::from_str(&t).map_err(|e| e.to_string())?;
                // The server greets every connection with its nonce before
                // any reply; only the answer to OUR request (id 1) decides.
                if v.get("id").is_none() && v.get("server_nonce").is_some() {
                    continue;
                }
                let _ = ws.close(None).await;
                return Ok(v);
            }
            Message::Close(_) => return Err("closed before answering".into()),
            _ => {}
        }
    }
    Err("closed before answering".into())
}

async fn tls_connect(
    cfg: &Config,
    authority: &str,
    tcp: TcpStream,
) -> Result<tokio_rustls::client::TlsStream<TcpStream>, String> {
    use tokio_rustls::rustls;
    let path = cfg.tls_cert.as_deref().ok_or("no certificate configured")?;
    let pem = std::fs::read(path).map_err(|e| format!("read {path}: {e}"))?;
    let pinned: Vec<Vec<u8>> = rustls_pemfile::certs(&mut &pem[..]).filter_map(|c| c.ok()).map(|c| c.as_ref().to_vec()).collect();
    let leaf = pinned.first().cloned().ok_or_else(|| format!("{path} holds no certificate"))?;
    let host = authority.rsplit_once(':').map(|(h, _)| h).unwrap_or(authority).trim_matches(['[', ']']).to_string();
    let name = rustls::pki_types::ServerName::try_from(host.clone()).map_err(|e| e.to_string())?;
    let provider = rustls::crypto::CryptoProvider::get_default().cloned()
        .unwrap_or_else(|| Arc::new(rustls::crypto::aws_lc_rs::default_provider()));
    let verifier = Arc::new(Pinned { leaf, host, provider: provider.clone() });
    let config = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions().map_err(|e| e.to_string())?
        .dangerous().with_custom_certificate_verifier(verifier).with_no_client_auth();
    tokio_rustls::TlsConnector::from(Arc::new(config)).connect(name, tcp).await.map_err(|e| e.to_string())
}

/// Trust exactly the configured certificate (pinned by its bytes — a
/// self-signed gateway cert, often a CA:TRUE one from the usual openssl
/// recipe, proves itself) AND require it to name the address dialled. Not
/// "any certificate": a different cert, or the right cert without the name,
/// fails, and the probe reports Occupied.
#[derive(Debug)]
struct Pinned {
    leaf: Vec<u8>,
    host: String,
    provider: Arc<tokio_rustls::rustls::crypto::CryptoProvider>,
}

impl tokio_rustls::rustls::client::danger::ServerCertVerifier for Pinned {
    fn verify_server_cert(
        &self,
        end_entity: &tokio_rustls::rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[tokio_rustls::rustls::pki_types::CertificateDer<'_>],
        _server_name: &tokio_rustls::rustls::pki_types::ServerName<'_>,
        _ocsp: &[u8],
        _now: tokio_rustls::rustls::pki_types::UnixTime,
    ) -> Result<tokio_rustls::rustls::client::danger::ServerCertVerified, tokio_rustls::rustls::Error> {
        use tokio_rustls::rustls::Error;
        if end_entity.as_ref() != self.leaf.as_slice() {
            return Err(Error::General("the gateway's certificate is not the configured one".into()));
        }
        if !cert_names(end_entity.as_ref(), &self.host) {
            return Err(Error::General(format!("the configured certificate does not name {}", self.host)));
        }
        Ok(tokio_rustls::rustls::client::danger::ServerCertVerified::assertion())
    }
    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &tokio_rustls::rustls::pki_types::CertificateDer<'_>,
        dss: &tokio_rustls::rustls::DigitallySignedStruct,
    ) -> Result<tokio_rustls::rustls::client::danger::HandshakeSignatureValid, tokio_rustls::rustls::Error> {
        tokio_rustls::rustls::crypto::verify_tls12_signature(message, cert, dss, &self.provider.signature_verification_algorithms)
    }
    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &tokio_rustls::rustls::pki_types::CertificateDer<'_>,
        dss: &tokio_rustls::rustls::DigitallySignedStruct,
    ) -> Result<tokio_rustls::rustls::client::danger::HandshakeSignatureValid, tokio_rustls::rustls::Error> {
        tokio_rustls::rustls::crypto::verify_tls13_signature(message, cert, dss, &self.provider.signature_verification_algorithms)
    }
    fn supported_verify_schemes(&self) -> Vec<tokio_rustls::rustls::SignatureScheme> {
        self.provider.signature_verification_algorithms.supported_schemes()
    }
}

/// Does the DER certificate name `host` in its subjectAltName (an IP
/// address entry for an IP, a DNS entry otherwise)? A small, exact reader of
/// the one extension (OID 2.5.29.17) — no CN fallback, no wildcards.
pub(crate) fn cert_names(der: &[u8], host: &str) -> bool {
    const SAN_OID: &[u8] = &[0x06, 0x03, 0x55, 0x1d, 0x11];
    let Some(at) = der.windows(SAN_OID.len()).position(|w| w == SAN_OID) else { return false };
    let mut i = at + SAN_OID.len();
    // optional critical BOOLEAN
    if der.get(i) == Some(&0x01) { i += 3; }
    // OCTET STRING wrapping the GeneralNames SEQUENCE
    let Some((_, body, _)) = tlv(der, i, 0x04) else { return false };
    let Some((_, seq, _)) = tlv(body, 0, 0x30) else { return false };
    let ip: Option<std::net::IpAddr> = host.parse().ok();
    let mut j = 0;
    while j < seq.len() {
        let tag = seq[j];
        let Some((_, value, next)) = tlv_any(seq, j) else { return false };
        match (tag, ip) {
            (0x87, Some(std::net::IpAddr::V4(a))) if value == a.octets() => return true,
            (0x87, Some(std::net::IpAddr::V6(a))) if value == a.octets() => return true,
            (0x82, None) if value.eq_ignore_ascii_case(host.as_bytes()) => return true,
            _ => {}
        }
        j = next;
    }
    false
}

fn tlv(buf: &[u8], at: usize, tag: u8) -> Option<(u8, &[u8], usize)> {
    (buf.get(at) == Some(&tag)).then_some(())?;
    tlv_any(buf, at)
}

fn tlv_any(buf: &[u8], at: usize) -> Option<(u8, &[u8], usize)> {
    let tag = *buf.get(at)?;
    let first = *buf.get(at + 1)? as usize;
    let (len, head) = if first < 0x80 {
        (first, 2)
    } else {
        let n = first & 0x7f;
        if n == 0 || n > 4 { return None; }
        let mut l = 0usize;
        for k in 0..n { l = (l << 8) | *buf.get(at + 2 + k)? as usize; }
        (l, 2 + n)
    };
    let start = at + head;
    let end = start.checked_add(len)?;
    Some((tag, buf.get(start..end)?, end))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(port: u16) -> Config {
        let mut c = Config::load();
        c.host = "0.0.0.0".into();
        c.port = port;
        c.token = "tok-ours".into();
        c.machine_id = "m-ours".into();
        c.tls_cert = None;
        c.tls_key = None;
        c
    }

    #[test]
    fn the_local_url_follows_the_bind_and_tls() {
        let mut c = cfg(9899);
        assert_eq!(local_url(&c), "ws://127.0.0.1:9899");
        c.host = "::".into();
        assert_eq!(local_url(&c), "ws://[::1]:9899");
        c.host = "100.64.0.7".into();
        assert_eq!(local_url(&c), "ws://100.64.0.7:9899", "a specific bind is dialled on itself");
        c.host = "fd00::2".into();
        assert_eq!(local_url(&c), "ws://[fd00::2]:9899");
        c.tls_cert = Some("/c.pem".into());
        c.tls_key = Some("/k.pem".into());
        assert!(local_url(&c).starts_with("wss://"));
    }

    #[test]
    fn the_verdict_is_ours_only_for_a_login_with_our_machine_id() {
        let url = "ws://127.0.0.1:1";
        let ok = |id: &str| serde_json::json!({ "id": 1, "result": { "authenticated": true, "machine_id": id } });
        assert!(matches!(judge(&ok("m-ours"), "m-ours", url), Verdict::Ours { .. }));
        assert!(matches!(judge(&ok("m-else"), "m-ours", url), Verdict::Occupied(_)), "another machine");
        assert!(matches!(judge(&ok(""), "m-ours", url), Verdict::Occupied(_)));
        let refused = serde_json::json!({ "id": 1, "error": { "code": -32001, "message": "invalid token" } });
        assert!(matches!(judge(&refused, "m-ours", url), Verdict::Occupied(_)), "a refused token is not free");
        assert!(matches!(judge(&serde_json::json!({ "hello": 1 }), "m-ours", url), Verdict::Occupied(_)));
    }

    /// A fake listener for each case, on a free loopback port.
    async fn listener() -> (tokio::net::TcpListener, u16) {
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let p = l.local_addr().unwrap().port();
        (l, p)
    }
    async fn ws_answering(answer: serde_json::Value) -> u16 {
        let (l, port) = listener().await;
        tokio::spawn(async move {
            let (s, _) = l.accept().await.unwrap();
            let mut ws = tokio_tungstenite::accept_async(s).await.unwrap();
            let _ = ws.send(Message::Text(serde_json::json!({ "server_nonce": "00", "e2e": 2 }).to_string().into())).await;
            let _ = ws.next().await;
            let _ = ws.send(Message::Text(answer.to_string().into())).await;
        });
        port
    }

    #[tokio::test]
    async fn the_probe_tells_ours_occupied_and_free_apart() {
        let ours = ws_answering(serde_json::json!({ "id": 1, "result": { "authenticated": true, "machine_id": "m-ours" } })).await;
        assert!(matches!(probe(&cfg(ours)).await, Verdict::Ours { .. }));
        let other = ws_answering(serde_json::json!({ "id": 1, "result": { "authenticated": true, "machine_id": "m-else" } })).await;
        assert!(matches!(probe(&cfg(other)).await, Verdict::Occupied(_)));
        let refused = ws_answering(serde_json::json!({ "id": 1, "error": { "code": -32001, "message": "invalid token" } })).await;
        assert!(matches!(probe(&cfg(refused)).await, Verdict::Occupied(_)));
        // A listener that is not a WebSocket at all.
        let (l, plain) = listener().await;
        tokio::spawn(async move {
            use tokio::io::AsyncWriteExt;
            let (mut s, _) = l.accept().await.unwrap();
            let _ = s.write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\n\r\nhi").await;
        });
        assert!(matches!(probe(&cfg(plain)).await, Verdict::Occupied(_)), "an HTTP server on the port is not free");
        // A listener that accepts and never answers.
        let (l, silent) = listener().await;
        tokio::spawn(async move {
            let (_s, _) = l.accept().await.unwrap();
            tokio::time::sleep(Duration::from_secs(5)).await;
        });
        match probe(&cfg(silent)).await {
            Verdict::Occupied(why) => assert!(why.contains("did not answer"), "{why}"),
            v => panic!("a silent listener is not free: {v:?}"),
        }
        // Nothing listening: the port of a listener we just dropped.
        let (l, free) = listener().await;
        drop(l);
        assert_eq!(probe(&cfg(free)).await, Verdict::None);
    }

    #[tokio::test]
    async fn a_tls_gateway_with_a_certificate_that_does_not_name_loopback_is_occupied_not_free() {
        // A configured cert that cannot be read or does not cover the address
        // dialled is never accepted and never reads as free.
        let (l, port) = listener().await;
        tokio::spawn(async move { let _ = l.accept().await; tokio::time::sleep(Duration::from_secs(3)).await; });
        let mut c = cfg(port);
        c.tls_cert = Some("/nonexistent/cert.pem".into());
        c.tls_key = Some("/nonexistent/key.pem".into());
        match probe(&c).await {
            Verdict::Occupied(why) => assert!(why.contains("TLS"), "{why}"),
            v => panic!("{v:?}"),
        }
    }

    /// A self-signed cert for `san`, written beside `dir` (openssl; skipped
    /// when the host has none).
    fn self_signed(dir: &std::path::Path, san: &str) -> Option<(String, String)> {
        let (c, k) = (dir.join("c.pem"), dir.join("k.pem"));
        let ok = std::process::Command::new("openssl")
            .args(["req", "-x509", "-newkey", "ec", "-pkeyopt", "ec_paramgen_curve:prime256v1", "-nodes", "-days", "1", "-subj", "/CN=gw",
                "-addext", &format!("subjectAltName={san}"), "-keyout"])
            .arg(&k).arg("-out").arg(&c)
            .stderr(std::process::Stdio::null()).stdout(std::process::Stdio::null())
            .status().map(|s| s.success()).unwrap_or(false);
        ok.then(|| (c.to_string_lossy().into(), k.to_string_lossy().into()))
    }

    /// A real TLS gateway (the server's own TLS path), trusted through the
    /// configured cert: Ours when the cert names 127.0.0.1, Occupied — never
    /// None, never a looser check — when it names something else.
    #[tokio::test]
    async fn a_tls_gateway_is_ours_only_when_its_trusted_certificate_names_the_address() {
        let guard = crate::tmux::Scratch::new("tlsprobe");
        for (san, ours) in [("IP:127.0.0.1", true), ("DNS:gateway.example", false), ("IP:127.0.0.1#other", false)] {
            let other_cert = san.ends_with("#other");
            let san = san.trim_end_matches("#other");
            let dir = std::path::Path::new(&guard.path()).join(format!("{san}-{other_cert}").replace([':', '.'], "_"));
            std::fs::create_dir_all(&dir).unwrap();
            let Some((cert, key)) = self_signed(&dir, san) else { eprintln!("no openssl — skipping"); return };
            let (l, port) = listener().await;
            drop(l);
            let (c2, k2) = (cert.clone(), key.clone());
            tokio::spawn(async move {
                let _ = crate::server::start_with_socket("127.0.0.1", port, "tok-ours", "m-ours", None, Some(c2), Some(k2), 5).await;
            });
            let mut c = cfg(port);
            c.host = "127.0.0.1".into();
            c.tls_key = Some(key);
            // The right name, but the client was configured with a different
            // certificate: pinning refuses it.
            c.tls_cert = Some(if other_cert {
                let d2 = dir.join("other");
                std::fs::create_dir_all(&d2).unwrap();
                self_signed(&d2, san).unwrap().0
            } else { cert });
            let mut v = Verdict::None;
            for _ in 0..40 {
                v = probe(&c).await;
                if v != Verdict::None { break; }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            if ours {
                assert!(matches!(v, Verdict::Ours { .. }), "{san}: {v:?}");
            } else {
                match v { Verdict::Occupied(why) => assert!(why.contains("TLS"), "{why}"), v => panic!("{san}: {v:?}") }
            }
        }
    }

    /// The real (plain) server: Ours with our token, Occupied with another
    /// token or another machine id.
    #[tokio::test]
    async fn the_real_server_is_ours_only_with_our_token_and_machine() {
        let (l, port) = listener().await;
        drop(l);
        tokio::spawn(async move {
            let _ = crate::server::start_with_socket("127.0.0.1", port, "tok-ours", "m-ours", None, None, None, 5).await;
        });
        let mut v = Verdict::None;
        for _ in 0..40 {
            v = probe(&cfg(port)).await;
            if v != Verdict::None { break; }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert!(matches!(v, Verdict::Ours { .. }), "{v:?}");
        let mut wrong = cfg(port);
        wrong.token = "tok-else".into();
        assert!(matches!(probe(&wrong).await, Verdict::Occupied(_)), "a refused token is not free");
        let mut other = cfg(port);
        other.machine_id = "m-else".into();
        assert!(matches!(probe(&other).await, Verdict::Occupied(_)), "our token, another machine id");
    }

    #[test]
    fn a_certificate_names_only_what_its_san_lists() {
        let guard = crate::tmux::Scratch::new("sanparse");
        let dir = std::path::Path::new(&guard.path()).to_path_buf();
        let Some((cert, _)) = self_signed(&dir, "IP:127.0.0.1,DNS:gw.local,IP:::1") else { eprintln!("no openssl — skipping"); return };
        let pem = std::fs::read(cert).unwrap();
        let der = rustls_pemfile::certs(&mut &pem[..]).next().unwrap().unwrap();
        assert!(cert_names(der.as_ref(), "127.0.0.1"));
        assert!(cert_names(der.as_ref(), "::1"));
        assert!(cert_names(der.as_ref(), "gw.local"));
        assert!(!cert_names(der.as_ref(), "127.0.0.2"));
        assert!(!cert_names(der.as_ref(), "gw"), "no CN fallback, no partial match");
        assert!(!cert_names(b"garbage", "127.0.0.1"));
    }
}

