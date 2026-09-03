//! Wire encoding for the WebSocket path: length-tagged deflate framing,
//! the token-derived AES-GCM session ciphers (v1 single key, v2 one key per
//! job), and the constant-time plain-token comparison. Split from server.rs
//! 2026-07-22.

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

/// Plain-token auth check. Constant time in the token's bytes: a `==` on
/// strings returns at the first differing byte, which lets a remote guesser
/// confirm a prefix one byte at a time by timing the rejection. The length
/// comparison short-circuits, which reveals only the token's length — the
/// configured token has a fixed length, so that leaks nothing useful.
pub(super) fn provided_token_matches(provided: &str, token: &str) -> bool {
    use subtle::ConstantTimeEq;
    !provided.is_empty()
        && provided.len() == token.len()
        && bool::from(provided.as_bytes().ct_eq(token.as_bytes()))
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

/// The current E2E protocol version. Advertised in the `server_nonce` frame,
/// requested by the client in its `auth` params, echoed in the authenticated
/// result. A peer that never mentions it speaks v1.
pub const E2E_VERSION: u64 = 2;

/// HKDF `info` labels. v1 has ONE label and used its output as the proof MAC
/// key AND the cipher key for both directions — so client→server message #n
/// and server→client message #n were sealed under the same (key, nonce),
/// which AES-GCM forbids (it leaks the XOR of the plaintexts and the GHASH
/// key). v2 keeps the same IKM (token) and salt (both nonces) and derives
/// three DISTINCT keys; the labels are what separate them, so they must
/// never be reused for anything else.
pub const E2E_V1_INFO: &[u8] = b"tmux-mobile-e2e";
pub const E2E_V2_INFO_PROOF: &[u8] = b"tmux-mobile-e2e/v2/proof";
pub const E2E_V2_INFO_C2S: &[u8] = b"tmux-mobile-e2e/v2/c2s";
pub const E2E_V2_INFO_S2C: &[u8] = b"tmux-mobile-e2e/v2/s2c";

fn hkdf_expand(token: &str, server_nonce: &[u8; 16], client_nonce: &[u8; 16], info: &[u8]) -> [u8; 32] {
    let mut salt = [0u8; 32];
    salt[..16].copy_from_slice(server_nonce);
    salt[16..].copy_from_slice(client_nonce);
    let hk = Hkdf::<Sha256>::new(Some(&salt), token.as_bytes());
    let mut key = [0u8; 32];
    // 32 bytes is far below HKDF-SHA256's 8160-byte limit; expand cannot fail.
    hk.expand(info, &mut key).unwrap();
    key
}

/// Derives the v1 session key (proof + both directions) from token + nonces.
/// Kept byte-for-byte so a v1 client keeps authenticating.
pub(super) fn derive_key(token: &str, server_nonce: &[u8; 16], client_nonce: &[u8; 16]) -> [u8; 32] {
    hkdf_expand(token, server_nonce, client_nonce, E2E_V1_INFO)
}

/// The three secrets of one session: what the client proves knowledge of,
/// what it encrypts with, what the server encrypts with. Each is used for
/// exactly one purpose, in exactly one direction.
pub struct SessionKeys {
    pub proof: [u8; 32],
    pub c2s: [u8; 32],
    pub s2c: [u8; 32],
}

/// Derives the v2 session keys. Same inputs as v1, three labels.
pub fn derive_session_keys(token: &str, server_nonce: &[u8; 16], client_nonce: &[u8; 16]) -> SessionKeys {
    SessionKeys {
        proof: hkdf_expand(token, server_nonce, client_nonce, E2E_V2_INFO_PROOF),
        c2s: hkdf_expand(token, server_nonce, client_nonce, E2E_V2_INFO_C2S),
        s2c: hkdf_expand(token, server_nonce, client_nonce, E2E_V2_INFO_S2C),
    }
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

    #[test]
    fn plain_token_match_is_exact_and_rejects_empty() {
        assert!(provided_token_matches("abc123", "abc123"));
        assert!(!provided_token_matches("", ""), "an empty token never authenticates");
        assert!(!provided_token_matches("", "abc123"));
        assert!(!provided_token_matches("abc12", "abc123"), "prefix is not a match");
        assert!(!provided_token_matches("abc1234", "abc123"));
        assert!(!provided_token_matches("abc124", "abc123"));
    }

    // ─── E2E v2 key separation ──────────────────────────────────────────

    const TEST_TOKEN: &str = "tmm-test-token";
    fn test_nonces() -> ([u8; 16], [u8; 16]) {
        let mut sn = [0u8; 16];
        let mut cn = [0u8; 16];
        for i in 0..16 {
            sn[i] = i as u8;
            cn[i] = 16 + i as u8;
        }
        (sn, cn)
    }

    /// Pinned against the SAME vectors in `src/lib/core/ws.test.ts`: the two
    /// implementations must agree byte-for-byte or no client can connect.
    #[test]
    fn e2e_key_derivation_matches_the_client_vectors() {
        let (sn, cn) = test_nonces();
        assert_eq!(
            bytes_to_hex(&derive_key(TEST_TOKEN, &sn, &cn)),
            "3ed67f3af05161bcc3b7dfa90cdac9f122073ca497a2ed17061ef8228628d1c3"
        );
        let k = derive_session_keys(TEST_TOKEN, &sn, &cn);
        assert_eq!(bytes_to_hex(&k.proof), "3991dde940bf280c1b61ab909e69fd55371b88e9fbbb4dca564baf47fca8c3aa");
        assert_eq!(bytes_to_hex(&k.c2s), "efb72f80e209dd1e24ea4e8091743df8c74e5d1105549bc1b8129d5e77061082");
        assert_eq!(bytes_to_hex(&k.s2c), "5dee4968bc587b108b14a7c9d8fb9442e48c35ced0a6538fb1ce5418e48ba07d");
    }

    #[test]
    fn e2e_v2_keys_are_pairwise_distinct_and_none_is_the_v1_key() {
        let (sn, cn) = test_nonces();
        let v1 = derive_key(TEST_TOKEN, &sn, &cn);
        let k = derive_session_keys(TEST_TOKEN, &sn, &cn);
        assert_ne!(k.proof, k.c2s);
        assert_ne!(k.proof, k.s2c);
        assert_ne!(k.c2s, k.s2c);
        assert_ne!(k.proof, v1);
        assert_ne!(k.c2s, v1);
        assert_ne!(k.s2c, v1);
    }

    /// The defect v2 exists to remove: under v1 both directions sealed their
    /// message #0 with the same (key, nonce). Under v2 the same plaintext at
    /// the same counter yields different ciphertext per direction.
    #[test]
    fn e2e_v2_directions_never_share_a_key_nonce_pair() {
        let (sn, cn) = test_nonces();
        let v1 = derive_key(TEST_TOKEN, &sn, &cn);
        let plaintext = encode_wire_payload(r#"{"id":1,"method":"ping","params":{}}"#);
        let a = HalfCipher::new(&v1).encrypt(&plaintext);
        let b = HalfCipher::new(&v1).encrypt(&plaintext);
        assert_eq!(a, b, "v1: same key, same counter, same ciphertext — the leak");

        let k = derive_session_keys(TEST_TOKEN, &sn, &cn);
        let c2s = HalfCipher::new(&k.c2s).encrypt(&plaintext);
        let s2c = HalfCipher::new(&k.s2c).encrypt(&plaintext);
        assert_ne!(c2s, s2c, "v2: distinct direction keys");
        // And each direction still round-trips with its own half.
        assert_eq!(HalfCipher::new(&k.c2s).decrypt(&c2s).unwrap(), plaintext);
        assert_eq!(HalfCipher::new(&k.s2c).decrypt(&s2c).unwrap(), plaintext);
        // A frame sealed for one direction is rejected by the other.
        assert!(HalfCipher::new(&k.s2c).decrypt(&c2s).is_err());
    }
}

