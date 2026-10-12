//! Unlocking the credential vault.
//!
//! The vault itself (bhterm-core::secrets) is AES-GCM under a PBKDF2 key and
//! never decides where that key comes from. This is where that policy lives.
//!
//! ⚠️ THE TRADE-OFF, stated plainly. The key is a random 32-byte file beside
//! the vault, owner-only, so the app unlocks silently and the user is never
//! prompted. That means anyone who can already read the user's own files can
//! read the key and therefore the secrets. It is NOT protection against
//! someone with your account; it protects against the file being copied
//! somewhere else — a backup, a synced folder, a shared drive — which is the
//! realistic exposure.
//!
//! The OS keychains were the alternative and were rejected for this family of
//! apps: a self-signed app looks like a NEW application to the macOS Keychain
//! after every update, so each build re-prompts, and because macOS authorises
//! reads per item a bulk operation became one password dialog per host.

use bhterm_core::SecretVault;
use rand::RngCore;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

fn key_file(data_dir: &Path) -> PathBuf {
    data_dir.join("vault.key")
}

/// Reads the key, creating one on first run. An unreadable key file is an
/// ERROR, never "no key" — generating a fresh one would make every stored
/// credential permanently undecryptable.
fn load_or_create_key(data_dir: &Path) -> io::Result<String> {
    let path = key_file(data_dir);
    match std::fs::read_to_string(&path) {
        Ok(key) if !key.trim().is_empty() => return Ok(key.trim().to_string()),
        Ok(_) => return Err(io::Error::other("the vault key file is empty")),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }

    let mut raw = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut raw);
    let key: String = raw.iter().map(|b| format!("{b:02x}")).collect();
    std::fs::write(&path, &key)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(key)
}

pub struct Vault {
    inner: Mutex<SecretVault>,
    path: PathBuf,
    key: String,
}

impl Vault {
    pub fn open(data_dir: &Path) -> Result<Self, String> {
        let key = load_or_create_key(data_dir).map_err(|e| format!("vault key: {e}"))?;
        let path = data_dir.join("secrets.json");
        let inner = SecretVault::load(&path, &key).map_err(|e| e.to_string())?;
        Ok(Vault { inner: Mutex::new(inner), path, key })
    }

    pub fn get(&self, account: &str) -> Option<String> {
        self.inner.lock().unwrap().get(account).map(|s| s.to_string())
    }

    pub fn set(&self, account: &str, secret: &str) -> Result<(), String> {
        let mut v = self.inner.lock().unwrap();
        v.set(account, secret);
        v.save(&self.path, &self.key).map_err(|e| e.to_string())
    }

    pub fn remove(&self, accounts: &[String]) -> Result<(), String> {
        let mut v = self.inner.lock().unwrap();
        let mut changed = false;
        for a in accounts {
            changed |= v.remove(a);
        }
        if changed {
            v.save(&self.path, &self.key).map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    /// Used by backup export/import, which is not wired up yet — kept here
    /// because the locking belongs with the vault, not with the backup code.
    #[allow(dead_code)]
    pub fn snapshot(&self) -> SecretVault {
        self.inner.lock().unwrap().clone()
    }

    #[allow(dead_code)]
    pub fn replace_from(&self, other: &SecretVault) -> Result<(), String> {
        let mut v = self.inner.lock().unwrap();
        for (account, secret) in other.all() {
            v.set(account.clone(), secret.clone());
        }
        v.save(&self.path, &self.key).map_err(|e| e.to_string())
    }
}

pub type SharedVault = Arc<Vault>;
