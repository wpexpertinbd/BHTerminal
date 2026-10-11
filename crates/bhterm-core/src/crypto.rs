//! AES-GCM under a PBKDF2-derived key.
//!
//! Deliberately byte-compatible with the shipped macOS app's CryptoKit use,
//! so a backup written by v1.x still opens here: PBKDF2-HMAC-SHA256 with a
//! 16-byte salt and 210k iterations (OWASP's floor), then AES-GCM whose
//! "combined" form is nonce ‖ ciphertext ‖ tag — the same layout CryptoKit's
//! `SealedBox.combined` produces.

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use rand::RngCore;
use sha2::Sha256;
use thiserror::Error;

pub const KDF_ALGORITHM: &str = "PBKDF2-HMAC-SHA256";
pub const KDF_ITERATIONS: u32 = 210_000;
const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 12;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum CryptoError {
    #[error("Couldn't encrypt — the backup was not written.")]
    EncryptionFailed,
    /// AES-GCM authenticates, so this covers a tampered file as well as a
    /// mistyped passphrase. Either way the contents can't be trusted.
    #[error("Wrong passphrase — the data couldn't be decrypted.")]
    WrongPassphrase,
    #[error("Malformed encrypted data.")]
    Malformed,
}

pub fn random_salt() -> Vec<u8> {
    let mut salt = vec![0u8; SALT_LEN];
    rand::thread_rng().fill_bytes(&mut salt);
    salt
}

pub fn derive_key(passphrase: &str, salt: &[u8], iterations: u32) -> [u8; 32] {
    let mut key = [0u8; 32];
    // Infallible for a non-zero output length.
    let _ = pbkdf2::pbkdf2::<hmac::Hmac<Sha256>>(passphrase.as_bytes(), salt, iterations, &mut key);
    key
}

/// Returns nonce ‖ ciphertext ‖ tag, base64-encoded.
pub fn seal(plaintext: &[u8], key: &[u8; 32]) -> Result<String, CryptoError> {
    let cipher = Aes256Gcm::new(key.into());
    let mut nonce_bytes = [0u8; NONCE_LEN];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ciphertext = cipher
        .encrypt(nonce, Payload { msg: plaintext, aad: &[] })
        .map_err(|_| CryptoError::EncryptionFailed)?;

    let mut combined = Vec::with_capacity(NONCE_LEN + ciphertext.len());
    combined.extend_from_slice(&nonce_bytes);
    combined.extend_from_slice(&ciphertext);
    Ok(B64.encode(combined))
}

pub fn open(sealed_b64: &str, key: &[u8; 32]) -> Result<Vec<u8>, CryptoError> {
    let combined = B64.decode(sealed_b64).map_err(|_| CryptoError::Malformed)?;
    if combined.len() <= NONCE_LEN {
        return Err(CryptoError::Malformed);
    }
    let (nonce_bytes, ciphertext) = combined.split_at(NONCE_LEN);
    let cipher = Aes256Gcm::new(key.into());
    cipher
        .decrypt(Nonce::from_slice(nonce_bytes), Payload { msg: ciphertext, aad: &[] })
        .map_err(|_| CryptoError::WrongPassphrase)
}

pub fn b64_encode(bytes: &[u8]) -> String {
    B64.encode(bytes)
}

pub fn b64_decode(s: &str) -> Result<Vec<u8>, CryptoError> {
    B64.decode(s).map_err(|_| CryptoError::Malformed)
}
