//! The credential vault: host passwords and key passphrases.
//!
//! ⚠️ Deliberately NOT the OS keychain. A self-signed BH app looks like a
//! different application to the macOS Keychain after every update, so each
//! build re-prompts — and because macOS authorises keychain reads *per item*,
//! an app holding one entry per host turned a backup export into one password
//! dialog per host. (Bulk reads are not an escape: the file-based keychain
//! rejects `kSecMatchLimitAll` together with `kSecReturnData`.) The same
//! applies in spirit to Windows Credential Manager and Secret Service, and
//! none of them behave alike.
//!
//! So: ONE encrypted file, owner-only, with the key derived from a passphrase
//! the shell supplies. The core never decides where that passphrase comes
//! from — that policy belongs to the shell.

use crate::crypto::{self, CryptoError};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum VaultError {
    #[error("{0}")]
    Crypto(#[from] CryptoError),
    #[error("Couldn't read or write the vault: {0}")]
    Io(String),
    #[error("The vault file is not in a format this version understands.")]
    Malformed,
}

const VAULT_FORMAT: &str = "bhterminal-vault";
const VAULT_VERSION: u32 = 1;

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct VaultFile {
    format: String,
    version: u32,
    kdf_salt: String,
    kdf_iterations: u32,
    sealed: String,
}

/// Account → secret. Kept in memory decrypted for the life of the session;
/// the file on disk is never plaintext.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SecretVault {
    entries: BTreeMap<String, String>,
}

impl SecretVault {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get(&self, account: &str) -> Option<&str> {
        self.entries.get(account).map(|s| s.as_str())
    }

    pub fn set(&mut self, account: impl Into<String>, secret: impl Into<String>) {
        self.entries.insert(account.into(), secret.into());
    }

    pub fn remove(&mut self, account: &str) -> bool {
        self.entries.remove(account).is_some()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn accounts(&self) -> impl Iterator<Item = &String> {
        self.entries.keys()
    }

    pub fn all(&self) -> &BTreeMap<String, String> {
        &self.entries
    }

    /// Absent file = empty vault, which is a first run — NOT an error, and
    /// never confused with "couldn't read it", because treating an unreadable
    /// vault as empty would overwrite every saved credential with nothing.
    pub fn load(path: &Path, passphrase: &str) -> Result<Self, VaultError> {
        let raw = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(e) => return Err(VaultError::Io(e.to_string())),
        };
        let file: VaultFile = serde_json::from_slice(&raw).map_err(|_| VaultError::Malformed)?;
        if file.format != VAULT_FORMAT || file.version > VAULT_VERSION {
            return Err(VaultError::Malformed);
        }
        let salt = crypto::b64_decode(&file.kdf_salt)?;
        let key = crypto::derive_key(passphrase, &salt, file.kdf_iterations);
        let plaintext = crypto::open(&file.sealed, &key)?;
        let entries: BTreeMap<String, String> =
            serde_json::from_slice(&plaintext).map_err(|_| VaultError::Malformed)?;
        Ok(SecretVault { entries })
    }

    pub fn save(&self, path: &Path, passphrase: &str) -> Result<(), VaultError> {
        let salt = crypto::random_salt();
        let key = crypto::derive_key(passphrase, &salt, crypto::KDF_ITERATIONS);
        let plaintext = serde_json::to_vec(&self.entries).map_err(|_| VaultError::Malformed)?;
        let sealed = crypto::seal(&plaintext, &key)?;

        let file = VaultFile {
            format: VAULT_FORMAT.into(),
            version: VAULT_VERSION,
            kdf_salt: crypto::b64_encode(&salt),
            kdf_iterations: crypto::KDF_ITERATIONS,
            sealed,
        };
        let json = serde_json::to_vec_pretty(&file).map_err(|_| VaultError::Malformed)?;

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| VaultError::Io(e.to_string()))?;
        }
        // Write-then-rename so a crash mid-write can't truncate the vault.
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, &json).map_err(|e| VaultError::Io(e.to_string()))?;
        restrict_to_owner(&tmp)?;
        std::fs::rename(&tmp, path).map_err(|e| VaultError::Io(e.to_string()))?;
        restrict_to_owner(path)?;
        Ok(())
    }
}

/// 0600. Encrypted or not, this file is the crown jewels — it must not be
/// readable by other accounts on the machine.
fn restrict_to_owner(path: &Path) -> Result<(), VaultError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
            .map_err(|e| VaultError::Io(e.to_string()))?;
    }
    #[cfg(not(unix))]
    {
        // Windows inherits the user-profile ACL, which is already owner-only;
        // there is no portable chmod to apply here.
        let _ = path;
    }
    Ok(())
}
