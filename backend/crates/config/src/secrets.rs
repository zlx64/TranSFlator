//! AES-256-GCM encryption for API keys at rest (§11).
//!
//! The key is derived as `SHA-256(APP_SECRET)`. Ciphertext is stored as
//! `base64(nonce || ciphertext+tag)`. Full keys are never logged.

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::Engine as _;
use rand::RngCore;
use sha2::{Digest, Sha256};

/// Errors from encrypt/decrypt operations.
#[derive(Debug, thiserror::Error)]
pub enum SecretError {
    #[error("ciphertext too short")]
    TooShort,
    #[error("invalid base64: {0}")]
    Base64(#[from] base64::DecodeError),
    #[error("cryptography error: {0}")]
    Crypto(String),
    #[error("decrypted bytes are not valid UTF-8")]
    NotUtf8,
}

/// Symmetric encryptor for secret settings values.
#[derive(Clone)]
pub struct Secrets {
    key: [u8; 32],
}

impl Secrets {
    /// Derive a [`Secrets`] handle from the `APP_SECRET` string.
    pub fn from_app_secret(app_secret: &str) -> Self {
        let digest = Sha256::digest(app_secret.as_bytes());
        let mut key = [0u8; 32];
        key.copy_from_slice(&digest);
        Self { key }
    }

    fn cipher(&self) -> Result<Aes256Gcm, SecretError> {
        Aes256Gcm::new_from_slice(&self.key).map_err(|e| SecretError::Crypto(e.to_string()))
    }

    /// Encrypt a plaintext secret, returning a base64 string.
    pub fn encrypt(&self, plaintext: &str) -> Result<String, SecretError> {
        let cipher = self.cipher()?;
        let mut nonce_bytes = [0u8; 12];
        rand::thread_rng().fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);

        let ciphertext = cipher
            .encrypt(nonce, plaintext.as_bytes())
            .map_err(|e| SecretError::Crypto(e.to_string()))?;

        let mut buf = Vec::with_capacity(12 + ciphertext.len());
        buf.extend_from_slice(&nonce_bytes);
        buf.extend_from_slice(&ciphertext);
        Ok(base64::engine::general_purpose::STANDARD.encode(&buf))
    }

    /// Decrypt a base64 string produced by [`Self::encrypt`].
    pub fn decrypt(&self, data: &str) -> Result<String, SecretError> {
        let buf = base64::engine::general_purpose::STANDARD.decode(data)?;
        if buf.len() < 12 {
            return Err(SecretError::TooShort);
        }
        let (nonce_bytes, ciphertext) = buf.split_at(12);
        let nonce = Nonce::from_slice(nonce_bytes);
        let cipher = self.cipher()?;
        let plaintext = cipher
            .decrypt(nonce, ciphertext)
            .map_err(|e| SecretError::Crypto(e.to_string()))?;
        String::from_utf8(plaintext).map_err(|_| SecretError::NotUtf8)
    }

    /// Mask a secret for display (show first 4 and last 4 chars).
    pub fn mask(value: &str) -> String {
        let b: Vec<char> = value.chars().collect();
        if b.len() <= 8 {
            return "****".to_string();
        }
        let head: String = b.iter().take(4).collect();
        let tail: String = b
            .iter()
            .rev()
            .take(4)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        format!("{head}…{tail}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let s = Secrets::from_app_secret("test-secret");
        let secret = "sk-1234567890abcdefghijklmnopqrstuvwxyz";
        let enc = s.encrypt(secret).unwrap();
        assert_ne!(enc, secret);
        let dec = s.decrypt(&enc).unwrap();
        assert_eq!(dec, secret);
    }

    #[test]
    fn different_secrets_cannot_decrypt() {
        let a = Secrets::from_app_secret("secret-a");
        let b = Secrets::from_app_secret("secret-b");
        let enc = a.encrypt("hello").unwrap();
        assert!(b.decrypt(&enc).is_err());
    }

    #[test]
    fn tamper_detected() {
        let s = Secrets::from_app_secret("secret");
        let enc = s.encrypt("hello world").unwrap();
        // Flip a byte in the middle (ciphertext region).
        let mut bytes = base64::engine::general_purpose::STANDARD
            .decode(&enc)
            .unwrap();
        let idx = bytes.len() / 2;
        bytes[idx] ^= 0xff;
        let tampered = base64::engine::general_purpose::STANDARD.encode(&bytes);
        assert!(s.decrypt(&tampered).is_err());
    }

    #[test]
    fn mask_hides_middle() {
        assert_eq!(Secrets::mask("sk-abc-def-1234"), "sk-a…1234");
        assert_eq!(Secrets::mask("short"), "****");
    }
}
