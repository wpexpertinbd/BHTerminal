//! The master-password lock on the vault key.
//!
//! The vault (secrets.rs) is sealed under a random key. Without a master
//! password that key sits beside it in a plain owner-only file. With one, the
//! key is ALSO sealed here under the master password, and the plain copy is
//! kept only if the user ticked "Remember on this computer".
//!
//! Wrapping the key rather than re-encrypting the vault under the password
//! means setting, changing or removing a master password never rewrites the
//! vault itself — the one file whose loss would cost every saved credential.

use crate::crypto::{self, CryptoError, MAX_ITERATIONS};
use serde::{Deserialize, Serialize};
use thiserror::Error;

const LOCK_FORMAT: &str = "bhterminal-vault-lock";
const LOCK_VERSION: u32 = 1;

/// Higher than the vault's own count: the vault is re-derived on every save,
/// this only once per unlock, so it can afford OWASP's current PBKDF2 figure.
pub const LOCK_ITERATIONS: u32 = 600_000;

pub const MIN_MASTER_LEN: usize = 8;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum LockError {
    #[error("Wrong master password.")]
    WrongPassword,
    #[error("The master password lock file is damaged or from a newer version.")]
    Malformed,
    #[error("Use a master password of at least {MIN_MASTER_LEN} characters.")]
    TooShort,
    #[error("Couldn't encrypt the vault key.")]
    EncryptionFailed,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LockFile {
    format: String,
    version: u32,
    kdf_algorithm: String,
    kdf_salt: String,
    kdf_iterations: u32,
    sealed: String,
}

/// Length is counted in characters, not bytes, so a Bengali password is not
/// held to a stricter rule than an English one.
pub fn check_master(master: &str) -> Result<(), LockError> {
    if master.chars().count() < MIN_MASTER_LEN {
        return Err(LockError::TooShort);
    }
    Ok(())
}

/// Seals `vault_key` under `master`, returning the lock file's bytes.
pub fn wrap(vault_key: &str, master: &str) -> Result<Vec<u8>, LockError> {
    check_master(master)?;
    let salt = crypto::random_salt();
    let key = crypto::derive_key(master, &salt, LOCK_ITERATIONS);
    let sealed = crypto::seal(vault_key.as_bytes(), &key).map_err(|_| LockError::EncryptionFailed)?;
    let file = LockFile {
        format: LOCK_FORMAT.into(),
        version: LOCK_VERSION,
        kdf_algorithm: crypto::KDF_ALGORITHM.into(),
        kdf_salt: crypto::b64_encode(&salt),
        kdf_iterations: LOCK_ITERATIONS,
        sealed,
    };
    serde_json::to_vec_pretty(&file).map_err(|_| LockError::EncryptionFailed)
}

/// Recovers the vault key. AES-GCM authenticates, so a wrong password and a
/// tampered file both fail here rather than yielding a wrong key.
pub fn unwrap(lock_bytes: &[u8], master: &str) -> Result<String, LockError> {
    let file: LockFile = serde_json::from_slice(lock_bytes).map_err(|_| LockError::Malformed)?;
    if file.format != LOCK_FORMAT
        || file.version > LOCK_VERSION
        || file.kdf_algorithm != crypto::KDF_ALGORITHM
        || file.kdf_iterations == 0
        || file.kdf_iterations > MAX_ITERATIONS
    {
        return Err(LockError::Malformed);
    }
    let salt = crypto::b64_decode(&file.kdf_salt).map_err(|_| LockError::Malformed)?;
    let key = crypto::derive_key(master, &salt, file.kdf_iterations);
    let plain = crypto::open(&file.sealed, &key).map_err(|e| match e {
        CryptoError::WrongPassphrase => LockError::WrongPassword,
        _ => LockError::Malformed,
    })?;
    String::from_utf8(plain).map_err(|_| LockError::Malformed)
}
