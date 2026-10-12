//! Backup export / restore.
//!
//! Format-compatible with the shipped macOS app's archive so a backup taken
//! from v1.x restores here: the same envelope, the same base64 fields, and
//! the same AES-GCM-under-PBKDF2 scheme (see crypto.rs).
//!
//! Secrets are opt-in and, when included, force encryption — they normally
//! live only in the vault, and writing them out in the clear would undo that.

use crate::crypto::{self, CryptoError};
use crate::model::{Host, HostFolder, Snippet};
use crate::secrets::SecretVault;
use crate::store::SessionStore;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use thiserror::Error;

pub const FORMAT: &str = "bhterminal-backup";
pub const VERSION: u32 = 1;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum BackupError {
    #[error("That file isn't a BHTerminal backup.")]
    NotABackup,
    #[error("This backup was written by a newer version of BHTerminal (format {0}). Update BHTerminal and try again.")]
    UnsupportedVersion(u32),
    #[error("This backup is encrypted and needs its passphrase.")]
    NeedsPassphrase,
    #[error("{0}")]
    Crypto(#[from] CryptoError),
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Payload {
    #[serde(default)]
    pub folders: Vec<HostFolder>,
    #[serde(default)]
    pub hosts: Vec<Host>,
    #[serde(default)]
    pub snippets: Vec<Snippet>,
    /// Vault account → secret. Only ever present in an encrypted archive.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secrets: Option<BTreeMap<String, String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Kdf {
    algorithm: String,
    salt: String,
    iterations: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Envelope {
    format: String,
    version: u32,
    exported_at: String,
    app_version: String,
    encrypted: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    kdf: Option<Kdf>,
    /// Plain archives carry the payload directly…
    #[serde(default, skip_serializing_if = "Option::is_none")]
    data: Option<Payload>,
    /// …encrypted ones carry a sealed box instead.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    sealed: Option<String>,
}

/// Gathers everything worth backing up. `include_secrets` pulls them from the
/// already-unlocked vault — the caller decides whether the user asked for it.
pub fn snapshot(store: &SessionStore, vault: Option<&SecretVault>) -> Payload {
    let mut payload = Payload {
        folders: store.folders.clone(),
        hosts: store.hosts.clone(),
        snippets: store.snippets.clone(),
        secrets: None,
    };
    if let Some(vault) = vault {
        // Only secrets belonging to hosts in THIS backup — never stale ones
        // left behind by deleted hosts.
        let wanted: Vec<String> = store
            .hosts
            .iter()
            .flat_map(|h| [h.secret_account_for(), h.passphrase_account_for()])
            .collect();
        let secrets: BTreeMap<String, String> = vault
            .all()
            .iter()
            .filter(|(k, v)| wanted.contains(k) && !v.is_empty())
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        payload.secrets = Some(secrets);
    }
    payload
}

pub fn encode(
    payload: &Payload,
    encrypt: bool,
    passphrase: Option<&str>,
    exported_at: &str,
    app_version: &str,
) -> Result<Vec<u8>, BackupError> {
    let mut payload = payload.clone();
    if !encrypt {
        payload.secrets = None;
    }

    let mut envelope = Envelope {
        format: FORMAT.into(),
        version: VERSION,
        exported_at: exported_at.into(),
        app_version: app_version.into(),
        encrypted: encrypt,
        kdf: None,
        data: None,
        sealed: None,
    };

    if encrypt {
        let passphrase = passphrase.filter(|p| !p.is_empty()).ok_or(BackupError::NeedsPassphrase)?;
        let salt = crypto::random_salt();
        let key = crypto::derive_key(passphrase, &salt, crypto::KDF_ITERATIONS);
        let plaintext = serde_json::to_vec(&payload).map_err(|_| CryptoError::EncryptionFailed)?;
        envelope.sealed = Some(crypto::seal(&plaintext, &key)?);
        envelope.kdf = Some(Kdf {
            algorithm: crypto::KDF_ALGORITHM.into(),
            salt: crypto::b64_encode(&salt),
            iterations: crypto::KDF_ITERATIONS,
        });
    } else {
        envelope.data = Some(payload);
    }

    serde_json::to_vec_pretty(&envelope).map_err(|_| BackupError::NotABackup)
}

fn parse(bytes: &[u8]) -> Result<Envelope, BackupError> {
    let envelope: Envelope = serde_json::from_slice(bytes).map_err(|_| BackupError::NotABackup)?;
    if envelope.format != FORMAT {
        return Err(BackupError::NotABackup);
    }
    if envelope.version > VERSION {
        return Err(BackupError::UnsupportedVersion(envelope.version));
    }
    Ok(envelope)
}

/// Whether a passphrase is needed before `decode` can succeed.
pub fn is_encrypted(bytes: &[u8]) -> Result<bool, BackupError> {
    Ok(parse(bytes)?.encrypted)
}

pub fn decode(bytes: &[u8], passphrase: Option<&str>) -> Result<Payload, BackupError> {
    let envelope = parse(bytes)?;
    if !envelope.encrypted {
        return envelope.data.ok_or(BackupError::NotABackup);
    }
    let passphrase = passphrase.filter(|p| !p.is_empty()).ok_or(BackupError::NeedsPassphrase)?;
    let (Some(kdf), Some(sealed)) = (envelope.kdf, envelope.sealed) else {
        return Err(BackupError::NotABackup);
    };
    let salt = crypto::b64_decode(&kdf.salt)?;
    let key = crypto::derive_key(passphrase, &salt, kdf.iterations);
    let plaintext = crypto::open(&sealed, &key)?;
    // A wrong passphrase fails the AEAD tag above, so anything that decrypts
    // but won't parse is a corrupt archive rather than a bad password.
    serde_json::from_slice(&plaintext).map_err(|_| CryptoError::WrongPassphrase.into())
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoreSummary {
    pub hosts_added: usize,
    pub hosts_updated: usize,
    pub folders_added: usize,
    pub folders_updated: usize,
    pub snippets_added: usize,
    pub snippets_updated: usize,
    pub secrets_restored: usize,
}

/// Merges by id: anything missing is added, anything present is updated, and
/// nothing is ever deleted — restoring an old backup must not throw away
/// hosts added since it was taken.
pub fn restore(
    payload: &Payload,
    store: &mut SessionStore,
    vault: Option<&mut SecretVault>,
) -> RestoreSummary {
    let mut summary = RestoreSummary::default();

    // Folders first, so every host has its folder to land in.
    for folder in &payload.folders {
        if store.update_folder(folder.clone()) {
            summary.folders_updated += 1;
        } else {
            store.folders.push(folder.clone());
            summary.folders_added += 1;
        }
    }
    for host in &payload.hosts {
        if store.update_host(host.clone()) {
            summary.hosts_updated += 1;
        } else {
            store.hosts.push(host.clone());
            summary.hosts_added += 1;
        }
    }
    for snippet in &payload.snippets {
        if store.update_snippet(snippet.clone()) {
            summary.snippets_updated += 1;
        } else {
            store.snippets.push(snippet.clone());
            summary.snippets_added += 1;
        }
    }
    if let (Some(vault), Some(secrets)) = (vault, payload.secrets.as_ref()) {
        for (account, secret) in secrets {
            vault.set(account.clone(), secret.clone());
            summary.secrets_restored += 1;
        }
    }
    summary
}
