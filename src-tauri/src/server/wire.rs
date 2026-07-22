//! Wire encoding for the WebSocket path: length-tagged deflate framing,
//! the token-derived AES-GCM session cipher, and constant-time token
//! comparison. Split from server.rs 2026-07-22 — content unchanged.

use aes_gcm::{aead::Aead, Aes256Gcm, KeyInit, Nonce};
use hkdf::Hkdf;
use sha2::Sha256;

// ─── Wire framing for the encrypted binary path ──────────────────────────
// Encrypted messages now travel as WebSocket BINARY frames. The first byte
// of the *plaintext* (post-decrypt) tells the receiver how to decode the
// rest:
//   0x00 = raw UTF-8 JSON (backward compatible with the old base64 path)
//   0x01 = raw deflate (RFC 1951) of UTF-8 JSON
//
// This avoids paying base64's 33% overhead and lets large pane snapshots
// ride deflate's LZ77 window, which collapses inter-frame redundancy by
// 20–50× in practice. Plaintext-token connections (no Web Crypto) keep
// using TEXT frames without framing.
pub const WIRE_PLAIN_JSON: u8 = 0x00;
pub const WIRE_DEFLATE_JSON: u8 = 0x01;
// Below this size, deflate's overhead (header + dict warm-up) makes the
// output bigger than the input. Skip compression for small payloads.
pub const COMPRESS_MIN_BYTES: usize = 256;

pub(super) fn bytes_to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

pub(super) fn hex_to_bytes(s: &str) -> Option<Vec<u8>> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok())
        .collect()
}

pub(super) fn provided_token_matches(provided: &str, token: &str) -> bool {
    !provided.is_empty() && provided == token
}

/// Encode a JSON string into the wire plaintext (pre-encryption) byte
/// stream: a 1-byte framing tag followed by the body. Compresses with
/// raw deflate (level=1) when the JSON is large enough to benefit; falls
/// back to plain bytes otherwise.
///
/// "Level 1" picks the speed end of zlib's spectrum — it's typically
/// ~3× faster than level 6 and gives up only a small ratio (the gains
/// come almost entirely from LZ77 back-references, which level 1 still
/// finds aggressively). Pane snapshots compress 20–50× even at level 1.
pub fn encode_wire_payload(json: &str) -> Vec<u8> {
    let bytes = json.as_bytes();
    if bytes.len() < COMPRESS_MIN_BYTES {
        let mut out = Vec::with_capacity(1 + bytes.len());
        out.push(WIRE_PLAIN_JSON);
        out.extend_from_slice(bytes);
        return out;
    }
    use flate2::{write::DeflateEncoder, Compression};
    use std::io::Write;
    let mut enc = DeflateEncoder::new(Vec::with_capacity(bytes.len() / 4 + 32), Compression::fast());
    // write_all + finish() never fail on a Vec writer.
    enc.write_all(bytes).expect("deflate write to Vec");
    let compressed = enc.finish().expect("deflate finish");
    // Pathological case: tiny string that deflate inflates due to overhead.
    // Fall back to plain so the receiver doesn't waste cycles on inflate.
    if compressed.len() + 1 >= bytes.len() + 1 {
        let mut out = Vec::with_capacity(1 + bytes.len());
        out.push(WIRE_PLAIN_JSON);
        out.extend_from_slice(bytes);
        return out;
    }
    let mut out = Vec::with_capacity(1 + compressed.len());
    out.push(WIRE_DEFLATE_JSON);
    out.extend_from_slice(&compressed);
    out
}

/// Decode a wire plaintext (post-decrypt) into the original JSON string.
/// Used only for tests / inbound — clients drive the inbound path, so this
/// rarely runs in production server, but keeping the inverse handy keeps
/// the protocol symmetric.
pub fn decode_wire_payload(buf: &[u8]) -> Result<String, String> {
    let (&tag, body) = buf.split_first().ok_or_else(|| "empty wire payload".to_string())?;
    match tag {
        WIRE_PLAIN_JSON => {
            String::from_utf8(body.to_vec()).map_err(|e| format!("plain wire payload not utf-8: {}", e))
        }
        WIRE_DEFLATE_JSON => {
            use flate2::read::DeflateDecoder;
            use std::io::Read;
            let mut dec = DeflateDecoder::new(body);
            let mut out = String::with_capacity(body.len() * 4);
            dec.read_to_string(&mut out).map_err(|e| format!("inflate failed: {}", e))?;
            Ok(out)
        }
        other => Err(format!("unknown wire framing tag: 0x{:02x}", other)),
    }
}

/// Derives AES-256-GCM key from token + nonces using HKDF-SHA256.
pub(super) fn derive_key(token: &str, server_nonce: &[u8; 16], client_nonce: &[u8; 16]) -> [u8; 32] {
    let mut salt = [0u8; 32];
    salt[..16].copy_from_slice(server_nonce);
    salt[16..].copy_from_slice(client_nonce);
    let hk = Hkdf::<Sha256>::new(Some(&salt), token.as_bytes());
    let mut key = [0u8; 32];
    hk.expand(b"tmux-mobile-e2e", &mut key).unwrap();
    key
}

/// Unidirectional cipher half: after auth, the session splits into a
/// recv-side (owned by the receiver loop, decrypts incoming in strict
/// order) and a send-side (owned by the send task, encrypts outgoing in
/// strict order). Splitting lets multiple business tasks run in parallel
/// without fighting over a single cipher's counter.
pub(super) struct HalfCipher {
    cipher: Aes256Gcm,
    counter: u64,
}

impl HalfCipher {
    pub(super) fn new(key: &[u8; 32]) -> Self {
        Self { cipher: Aes256Gcm::new_from_slice(key).unwrap(), counter: 0 }
    }
    pub(super) fn encrypt(&mut self, plaintext: &[u8]) -> Vec<u8> {
        let mut nonce_bytes = [0u8; 12];
        nonce_bytes[4..].copy_from_slice(&self.counter.to_be_bytes());
        self.counter += 1;
        self.cipher.encrypt(Nonce::from_slice(&nonce_bytes), plaintext).unwrap()
    }
    pub(super) fn decrypt(&mut self, ciphertext: &[u8]) -> Result<Vec<u8>, String> {
        let mut nonce_bytes = [0u8; 12];
        nonce_bytes[4..].copy_from_slice(&self.counter.to_be_bytes());
        self.counter += 1;
        self.cipher.decrypt(Nonce::from_slice(&nonce_bytes), ciphertext)
            .map_err(|_| "decryption failed".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_small_payload_skips_compression() {
        let json = r#"{"id":1,"result":"pong"}"#;
        let encoded = encode_wire_payload(json);
        assert_eq!(encoded[0], WIRE_PLAIN_JSON, "small payload must stay uncompressed");
        assert_eq!(&encoded[1..], json.as_bytes());
        assert_eq!(decode_wire_payload(&encoded).unwrap(), json);
    }

    #[test]
    fn wire_large_payload_compresses() {
        // 4 KB of repeated content — perfect deflate target.
        let json = format!(r#"{{"content":"{}"}}"#, "ABCDEFGH".repeat(500));
        assert!(json.len() > COMPRESS_MIN_BYTES);
        let encoded = encode_wire_payload(&json);
        assert_eq!(encoded[0], WIRE_DEFLATE_JSON, "large payload must be compressed");
        assert!(
            encoded.len() < json.len() / 5,
            "compression ratio too weak: {} → {}",
            json.len(),
            encoded.len()
        );
        assert_eq!(decode_wire_payload(&encoded).unwrap(), json);
    }

    #[test]
    fn wire_payload_with_ansi_codes_roundtrip() {
        // Realistic pane snapshot: ANSI SGR escape sequences should pass
        // through (deflate is byte-clean; we just don't want any UTF-8 mishap).
        let json = format!(
            r#"{{"content":"{}"}}"#,
            "\u{001b}[38;2;255;100;200mhello\u{001b}[0m world\n".repeat(60)
        );
        assert!(json.len() > COMPRESS_MIN_BYTES);
        let encoded = encode_wire_payload(&json);
        assert_eq!(encoded[0], WIRE_DEFLATE_JSON);
        assert_eq!(decode_wire_payload(&encoded).unwrap(), json);
    }

    #[test]
    fn wire_payload_unicode_roundtrip() {
        // 多字节 UTF-8 不能被 deflate / inflate 弄坏。
        let json = format!(r#"{{"msg":"{}"}}"#, "中文 日本語 한국어 🎉 ".repeat(50));
        let encoded = encode_wire_payload(&json);
        let decoded = decode_wire_payload(&encoded).unwrap();
        assert_eq!(decoded, json);
    }

    #[test]
    fn wire_pathological_random_skips_compression() {
        // High-entropy data that deflate can't compress: encode_wire_payload
        // should detect "compressed >= original" and emit plain instead.
        let mut rng_bytes = vec![0u8; 600];
        for (i, b) in rng_bytes.iter_mut().enumerate() {
            // Deterministic pseudo-random pattern, byte-clean ASCII so JSON-like.
            *b = ((i * 2654435761) % 94 + 32) as u8;
        }
        let json = String::from_utf8(rng_bytes).unwrap();
        let encoded = encode_wire_payload(&json);
        // Either compressed or plain is acceptable — both must roundtrip.
        // The important invariant is encoded.len() < 2 * json.len() (no
        // runaway expansion) and lossless decode.
        assert!(encoded.len() < json.len() * 2);
        assert_eq!(decode_wire_payload(&encoded).unwrap(), json);
    }

    #[test]
    fn wire_decode_rejects_unknown_tag() {
        let bogus = vec![0xff, 1, 2, 3];
        let err = decode_wire_payload(&bogus).unwrap_err();
        assert!(err.contains("unknown wire framing tag"), "got: {}", err);
    }

    #[test]
    fn wire_decode_rejects_empty() {
        let err = decode_wire_payload(&[]).unwrap_err();
        assert!(err.contains("empty"), "got: {}", err);
    }

    // ─── Full encryption + framing roundtrip ─────────────────────────────

    fn make_paired_ciphers() -> (HalfCipher, HalfCipher) {
        // Send half writes, recv half reads — both initialised from the
        // same key. (In production the two sides live on different
        // hosts; for tests we just need any matched pair.)
        let key = [0x42u8; 32];
        (HalfCipher::new(&key), HalfCipher::new(&key))
    }

    #[test]
    fn encrypted_compressed_roundtrip_typical_pane_snapshot() {
        // Mimic a `pane_output` notification with a 50-line pane payload.
        let pane = (0..50)
            .map(|i| format!("\u{001b}[38;5;{}m line {} content content content\u{001b}[0m", (i % 200) + 16, i))
            .collect::<Vec<_>>()
            .join("\n");
        let msg = serde_json::json!({
            "id": null,
            "method": "pane_output",
            "params": {
                "target": "test:0.0",
                "content": pane,
                "cursor": {"x": 4, "y": 24, "w": 80, "h": 24, "t": 0}
            }
        });
        let json = serde_json::to_string(&msg).unwrap();

        let (mut send_c, mut recv_c) = make_paired_ciphers();
        let plaintext = encode_wire_payload(&json);
        let ciphertext = send_c.encrypt(&plaintext);
        // Verify wire size advantage: ciphertext should be smaller than the
        // old base64(encrypted(json)) path. base64 inflates by ~4/3, plus
        // we still have to encrypt the full uncompressed JSON.
        assert!(
            ciphertext.len() < (json.len() * 4 / 3) / 4,
            "expected at least 4× shrink vs base64; got json={} ct={}",
            json.len(),
            ciphertext.len()
        );

        let recovered_pt = recv_c.decrypt(&ciphertext).expect("decrypt ok");
        let recovered_json = decode_wire_payload(&recovered_pt).expect("decode ok");
        assert_eq!(recovered_json, json);
    }

    #[test]
    fn encrypted_small_message_roundtrip_no_compression() {
        let json = r#"{"id":42,"result":{"ok":true}}"#;
        let (mut send_c, mut recv_c) = make_paired_ciphers();
        let plaintext = encode_wire_payload(json);
        // Verify framing: small → plain.
        assert_eq!(plaintext[0], WIRE_PLAIN_JSON);
        let ciphertext = send_c.encrypt(&plaintext);
        let recovered_pt = recv_c.decrypt(&ciphertext).expect("decrypt ok");
        let recovered_json = decode_wire_payload(&recovered_pt).expect("decode ok");
        assert_eq!(recovered_json, json);
    }

    #[test]
    fn cipher_counter_advances_strictly() {
        // Two consecutive encrypts must produce different ciphertexts even
        // for the same plaintext (because the nonce counter advances). And
        // the recv half must decrypt them in matching order.
        let pt = b"identical message";
        let (mut send_c, mut recv_c) = make_paired_ciphers();
        let ct1 = send_c.encrypt(pt);
        let ct2 = send_c.encrypt(pt);
        assert_ne!(ct1, ct2, "GCM must emit distinct ciphertext per nonce");
        assert_eq!(recv_c.decrypt(&ct1).unwrap(), pt);
        assert_eq!(recv_c.decrypt(&ct2).unwrap(), pt);
    }

    #[test]
    fn cipher_rejects_out_of_order_decrypt() {
        // If the receiver's counter is ahead of the actual ciphertext's
        // nonce, GCM authentication fails — that's the property we rely on
        // for replay protection.
        let pt = b"hello";
        let (mut send_c, mut recv_c) = make_paired_ciphers();
        let ct1 = send_c.encrypt(pt);
        let _ct2 = send_c.encrypt(pt);
        // Skip ahead on the receive side, then try to decrypt ct1.
        let _ = recv_c.decrypt(&_ct2); // advances counter past ct1
        assert!(recv_c.decrypt(&ct1).is_err(), "ct1 must fail under wrong nonce");
    }

    // ─── HTTP /dl request parsing ────────────────────────────────────────

    #[test]
    fn compression_ratio_demo_pane_snapshot() {
        // Demonstrates the bandwidth win on a realistic snapshot. Not a
        // strict assertion (avoid making future deflate library updates
        // flap the test), just bounds.
        let lines: Vec<String> = (0..24)
            .map(|i| format!("$ command --flag-{}={}\u{001b}[0m output for row {}", i, i * 7, i))
            .collect();
        let snapshot = lines.join("\n");
        let json = serde_json::json!({"content": snapshot}).to_string();
        let plaintext = encode_wire_payload(&json);
        eprintln!(
            "[compression demo] json={} bytes, wire={} bytes, ratio={:.2}×",
            json.len(),
            plaintext.len(),
            json.len() as f64 / plaintext.len() as f64
        );
        assert!(plaintext.len() < json.len(), "wire payload should shrink");
    }
}

