//! Cryptographic helpers: password hashing, token generation and hashing,
//! authenticated encryption of stored secrets, and HMAC utilities.

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use argon2::password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::{Algorithm, Argon2, Params, Version};
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use base64::Engine;
use hmac::{Hmac, Mac};
use rand::RngCore;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

/// `n` random bytes, URL-safe base64 without padding.
pub fn random_token(n: usize) -> String {
    let mut buf = vec![0u8; n];
    rand::thread_rng().fill_bytes(&mut buf);
    URL_SAFE_NO_PAD.encode(buf)
}

/// Lowercase hex of `n` random bytes.
pub fn random_hex(n: usize) -> String {
    let mut buf = vec![0u8; n];
    rand::thread_rng().fill_bytes(&mut buf);
    hex::encode(buf)
}

pub fn sha256(data: &[u8]) -> Vec<u8> {
    Sha256::digest(data).to_vec()
}

pub fn sha256_hex(data: &[u8]) -> String {
    hex::encode(Sha256::digest(data))
}

pub fn hmac_sha256(key: &[u8], msg: &[u8]) -> Vec<u8> {
    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(key).expect("HMAC accepts any key length");
    mac.update(msg);
    mac.finalize().into_bytes().to_vec()
}

/// Constant-time equality for secrets and signatures.
pub fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && bool::from(a.ct_eq(b))
}

fn argon2() -> Argon2<'static> {
    // OWASP minimum for Argon2id: 19 MiB, 2 iterations, 1 lane.
    Argon2::new(Algorithm::Argon2id, Version::V0x13, Params::new(19_456, 2, 1, None).expect("valid argon2 params"))
}

/// Hash a password with Argon2id. CPU-heavy: call from `spawn_blocking`.
pub fn hash_password(password: &str) -> String {
    let salt = SaltString::generate(&mut OsRng);
    argon2().hash_password(password.as_bytes(), &salt).expect("argon2 hashing cannot fail with valid params").to_string()
}

/// Verify a password against a PHC string. CPU-heavy: call from `spawn_blocking`.
pub fn verify_password(password: &str, phc: &str) -> bool {
    match PasswordHash::new(phc) {
        Ok(parsed) => argon2().verify_password(password.as_bytes(), &parsed).is_ok(),
        Err(_) => false,
    }
}

/// A hash that never verifies, used to equalise timing when the account does not exist.
pub fn dummy_hash() -> &'static str {
    static DUMMY: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    DUMMY.get_or_init(|| hash_password("gantry-timing-equaliser"))
}

/// AES-256-GCM box for secrets stored in the database.
#[derive(Clone)]
pub struct SecretBox {
    cipher: Aes256Gcm,
}

impl SecretBox {
    /// `key_b64` is base64 of exactly 32 bytes.
    pub fn new(key_b64: &str) -> anyhow::Result<Self> {
        let key = STANDARD
            .decode(key_b64.trim())
            .or_else(|_| URL_SAFE_NO_PAD.decode(key_b64.trim()))
            .map_err(|_| anyhow::anyhow!("security.secret_key is not valid base64"))?;
        anyhow::ensure!(key.len() == 32, "security.secret_key must decode to 32 bytes, got {}", key.len());
        Ok(Self { cipher: Aes256Gcm::new_from_slice(&key).expect("32-byte key") })
    }

    pub fn generate_key() -> String {
        let mut buf = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut buf);
        STANDARD.encode(buf)
    }

    /// Encrypt; the result is `v1:` + base64(nonce || ciphertext). Empty input stays empty.
    pub fn seal(&self, plaintext: &str) -> String {
        if plaintext.is_empty() {
            return String::new();
        }
        let mut nonce = [0u8; 12];
        rand::thread_rng().fill_bytes(&mut nonce);
        let ct = self.cipher.encrypt(Nonce::from_slice(&nonce), plaintext.as_bytes()).expect("AES-GCM encryption cannot fail");
        let mut out = nonce.to_vec();
        out.extend(ct);
        format!("v1:{}", STANDARD.encode(out))
    }

    /// Decrypt a value produced by [`seal`]. Returns `None` if it was tampered with or the key changed.
    pub fn open(&self, sealed: &str) -> Option<String> {
        if sealed.is_empty() {
            return Some(String::new());
        }
        let raw = STANDARD.decode(sealed.strip_prefix("v1:")?).ok()?;
        if raw.len() < 13 {
            return None;
        }
        let (nonce, ct) = raw.split_at(12);
        let pt = self.cipher.decrypt(Nonce::from_slice(nonce), ct).ok()?;
        String::from_utf8(pt).ok()
    }
}

/// API token: `gty_<prefix>_<secret>`. Only the prefix and a SHA-256 of the secret are stored.
pub struct NewToken {
    pub full: String,
    pub prefix: String,
    pub secret_hash: Vec<u8>,
}

pub fn new_api_token() -> NewToken {
    let prefix = random_hex(5);
    let secret = random_token(32);
    NewToken { full: format!("gty_{prefix}_{secret}"), secret_hash: sha256(secret.as_bytes()), prefix }
}

/// Split a presented token into (prefix, secret). Rejects anything not shaped like a Gantry token.
pub fn parse_api_token(token: &str) -> Option<(&str, &str)> {
    let rest = token.strip_prefix("gty_")?;
    let (prefix, secret) = rest.split_once('_')?;
    (prefix.len() == 10 && prefix.bytes().all(|b| b.is_ascii_hexdigit()) && secret.len() >= 40 && secret.len() <= 64).then_some((prefix, secret))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_round_trip() {
        let h = hash_password("correct horse battery staple");
        assert!(h.starts_with("$argon2id$"));
        assert!(verify_password("correct horse battery staple", &h));
        assert!(!verify_password("correct horse battery stapl", &h));
        assert!(!verify_password("x", "not-a-hash"));
    }

    #[test]
    fn same_password_hashes_differently() {
        assert_ne!(hash_password("same-password-123"), hash_password("same-password-123"));
    }

    #[test]
    fn secret_box_round_trip_and_tamper() {
        let sb = SecretBox::new(&SecretBox::generate_key()).unwrap();
        let sealed = sb.seal("client-secret");
        assert!(sealed.starts_with("v1:") && !sealed.contains("client-secret"));
        assert_eq!(sb.open(&sealed).as_deref(), Some("client-secret"));
        let mut bytes = STANDARD.decode(&sealed[3..]).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 1;
        assert_eq!(sb.open(&format!("v1:{}", STANDARD.encode(bytes))), None);
        let other = SecretBox::new(&SecretBox::generate_key()).unwrap();
        assert_eq!(other.open(&sealed), None);
        assert_eq!(sb.seal(""), "");
        assert_eq!(sb.open("").as_deref(), Some(""));
    }

    #[test]
    fn secret_box_rejects_bad_keys() {
        assert!(SecretBox::new("short").is_err());
        assert!(SecretBox::new(&STANDARD.encode([0u8; 16])).is_err());
    }

    #[test]
    fn api_token_shape() {
        let t = new_api_token();
        let (prefix, secret) = parse_api_token(&t.full).unwrap();
        assert_eq!(prefix, t.prefix);
        assert_eq!(sha256(secret.as_bytes()), t.secret_hash);
        assert!(parse_api_token("gty_short_x").is_none());
        assert!(parse_api_token("Bearer abc").is_none());
        assert!(parse_api_token("gty_zzzzzzzzzz_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").is_none());
    }

    #[test]
    fn hmac_known_answer() {
        // RFC 4231 test case 2.
        let mac = hmac_sha256(b"Jefe", b"what do ya want for nothing?");
        assert_eq!(hex::encode(mac), "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843");
    }

    #[test]
    fn constant_time_eq() {
        assert!(ct_eq(b"abc", b"abc"));
        assert!(!ct_eq(b"abc", b"abd"));
        assert!(!ct_eq(b"abc", b"abcd"));
    }
}
