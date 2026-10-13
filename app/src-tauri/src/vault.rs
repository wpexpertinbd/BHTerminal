//! Unlocking the credential vault.
//!
//! The vault itself (bhterm-core::secrets) is AES-GCM under a PBKDF2 key and
//! never decides where that key comes from. This is where that policy lives.
//!
//! The key is a random 32-byte value. These files decide how it is reached:
//!
//! - `vault.key`  — the key in plain, owner-only. Present when there is no
//!   master password, or when there is one and "Remember on this computer"
//!   is ticked. With it the app unlocks silently.
//! - `vault.lock` — the key sealed under the master password (core::lock).
//!   Present whenever a master password is set.
//! - `vault.key.next` / `vault.lock.next` — a NEW key being switched to. See
//!   `rotate`: they exist only between staging a new key and finishing.
//! - `master-password-asked` — marks that the one-time "set a master
//!   password?" question has been answered, so it is never asked twice.
//!
//! ⚠️ THE TRADE-OFF, stated plainly. While `vault.key` exists, anyone who can
//! already read the user's own files can read the secrets. That protects
//! against the vault being copied somewhere else — a backup, a synced folder
//! — but not against someone with the account. Only a master password WITHOUT
//! "remember" closes that, and then a forgotten password cannot be recovered.
//!
//! The OS keychains were the alternative and were rejected for this family of
//! apps: a self-signed app looks like a NEW application to the macOS Keychain
//! after every update, so each build re-prompts, and because macOS authorises
//! reads per item a bulk operation became one password dialog per host.

use bhterm_core::crypto::CryptoError;
use bhterm_core::lock::{self, LockError};
use bhterm_core::secrets::VaultError;
use bhterm_core::SecretVault;
use rand::RngCore;
use serde::Serialize;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

const LOCKED: &str = "Saved passwords are locked — unlock BHTerminal with your master password first.";

fn key_file(data_dir: &Path) -> PathBuf {
    data_dir.join("vault.key")
}

fn lock_file(data_dir: &Path) -> PathBuf {
    data_dir.join("vault.lock")
}

fn next_key_file(data_dir: &Path) -> PathBuf {
    data_dir.join("vault.key.next")
}

fn next_lock_file(data_dir: &Path) -> PathBuf {
    data_dir.join("vault.lock.next")
}

fn asked_file(data_dir: &Path) -> PathBuf {
    data_dir.join("master-password-asked")
}

/// Each target gets its OWN temp file, so two writes can never collide on
/// one, and a crash leaves behind a file whose name says what it held.
fn tmp_for(path: &Path) -> PathBuf {
    let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    path.with_file_name(format!("{name}.tmp"))
}

/// Reads a plain key. Absent = Ok(None); unreadable or empty is an ERROR,
/// never "no key" — treating it as absent would lead to a fresh key and make
/// every stored credential permanently undecryptable.
fn read_key_at(path: &Path) -> io::Result<Option<String>> {
    match std::fs::read_to_string(path) {
        Ok(key) if !key.trim().is_empty() => Ok(Some(key.trim().to_string())),
        Ok(_) => Err(io::Error::other("the vault key file is empty")),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

fn new_key() -> String {
    let mut raw = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut raw);
    raw.iter().map(|b| format!("{b:02x}")).collect()
}

/// Write-then-rename, owner-only from the moment the file exists — a crash
/// mid-write must not leave a truncated key or lock behind, and there must be
/// no instant where the key is readable by other accounts.
fn write_private(path: &Path, bytes: &[u8]) -> io::Result<()> {
    use std::io::Write;
    let tmp = tmp_for(path);
    let _ = std::fs::remove_file(&tmp);
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut f = options.open(&tmp)?;
    f.write_all(bytes)?;
    f.sync_all()?;
    drop(f);
    std::fs::rename(&tmp, path)
}

fn remove_if_present(path: &Path) -> io::Result<()> {
    match std::fs::remove_file(path) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

fn io_err(what: &str, e: io::Error) -> String {
    format!("{what}: {e}")
}

/// Opens the vault with `key`. Ok(None) means "not this key" — AES-GCM's tag
/// failed — as distinct from a damaged or unreadable file, which is an error
/// and must never be mistaken for a key that merely did not fit.
fn try_open(path: &Path, key: &str) -> Result<Option<SecretVault>, String> {
    match SecretVault::load(path, key) {
        Ok(v) => Ok(Some(v)),
        Err(VaultError::Crypto(CryptoError::WrongPassphrase)) => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

enum State {
    Locked,
    Unlocked {
        secrets: SecretVault,
        key: String,
        /// Held in memory rather than read off the disk at each check:
        /// deleting `vault.lock` while the app is open must not quietly
        /// turn the master-password checks off.
        has_master: bool,
    },
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VaultStatus {
    pub locked: bool,
    pub has_master: bool,
    /// A master password is set AND this computer keeps the key.
    pub remembered: bool,
    /// The one-time "set a master password?" question is still to be asked.
    pub ask_setup: bool,
}

/// When the vault file was last seen: how another BHTerminal process is
/// noticed. Modified time AND length, because a coarse clock can give two
/// quick saves the same time.
type Stamp = Option<(std::time::SystemTime, u64)>;

pub struct Vault {
    state: Mutex<State>,
    path: PathBuf,
    data_dir: PathBuf,
    /// The file as this process last read or wrote it. Always touched with
    /// `state` held, in that order.
    stamp: Mutex<Stamp>,
}

fn stamp_of(path: &Path) -> Stamp {
    let meta = std::fs::metadata(path).ok()?;
    Some((meta.modified().ok()?, meta.len()))
}

impl Vault {
    pub fn open(data_dir: &Path) -> Result<Self, String> {
        let path = data_dir.join("secrets.json");
        // A temp file left by a crash may hold a plain key; it has no use.
        for f in [key_file(data_dir), next_key_file(data_dir), lock_file(data_dir), next_lock_file(data_dir)] {
            let _ = std::fs::remove_file(tmp_for(&f));
        }
        let vault = |state| Vault {
            state: Mutex::new(state),
            path: path.clone(),
            data_dir: data_dir.to_path_buf(),
            stamp: Mutex::new(stamp_of(&path)),
        };
        let has_lock = lock_file(data_dir).exists() || next_lock_file(data_dir).exists();

        if let Some(key) = read_key_at(&key_file(data_dir)).map_err(|e| io_err("vault key", e))? {
            if let Some(secrets) = try_open(&path, &key)? {
                // The current key opens it: any staged key belongs to a
                // switch that never reached the vault. Drop it.
                let v = vault(State::Unlocked { secrets, key, has_master: false });
                v.discard_staged();
                let has_master = lock_file(data_dir).exists();
                if let State::Unlocked { has_master: m, .. } = &mut *v.state.lock().unwrap() {
                    *m = has_master;
                }
                return Ok(v);
            }
            // The vault moved to a new key and the switch was interrupted
            // after the vault was saved: the staged key is the right one.
            if let Some(next) = read_key_at(&next_key_file(data_dir)).map_err(|e| io_err("vault key", e))? {
                if let Some(secrets) = try_open(&path, &next)? {
                    let v = vault(State::Unlocked { secrets, key: next, has_master: true });
                    v.promote_staged().map_err(|e| io_err("vault key", e))?;
                    return Ok(v);
                }
            }
            // A stale plain key left by an interrupted switch to an
            // UNremembered password: the lock decides, so ask for it.
            if has_lock {
                return Ok(vault(State::Locked));
            }
            return Err("the vault key does not open the saved passwords".into());
        }

        if has_lock {
            return Ok(vault(State::Locked));
        }
        // Neither key nor lock. A vault file with no way to open it is NOT a
        // first run, and starting empty would overwrite it.
        if path.exists() {
            return Err("the vault exists but its key is missing".into());
        }
        let key = new_key();
        write_private(&key_file(data_dir), key.as_bytes()).map_err(|e| io_err("vault key", e))?;
        Ok(vault(State::Unlocked { secrets: SecretVault::new(), key, has_master: false }))
    }

    pub fn status(&self) -> VaultStatus {
        let (locked, has_master) = match &*self.state.lock().unwrap() {
            State::Locked => (true, true),
            State::Unlocked { has_master, .. } => (false, *has_master),
        };
        VaultStatus {
            locked,
            has_master,
            remembered: has_master && !locked && key_file(&self.data_dir).exists(),
            ask_setup: !has_master && !asked_file(&self.data_dir).exists(),
        }
    }

    /// Saves, and remembers that the file now on disk is ours.
    fn save_secrets(&self, secrets: &SecretVault, key: &str) -> Result<(), String> {
        secrets.save(&self.path, key).map_err(|e| e.to_string())?;
        *self.stamp.lock().unwrap() = stamp_of(&self.path);
        Ok(())
    }

    /// Picks up what ANOTHER BHTerminal process saved since this one last
    /// looked.
    ///
    /// Two copies can run against one data folder (an old copy still in the
    /// tray, a second launch before the single-instance guard). Each holds
    /// the vault in memory, so without this a password saved in one is
    /// invisible to the other — "it didn't keep my password" — and the other's
    /// next save writes its stale picture over it, ERASING it. If the file
    /// changed but this process's key no longer opens it (the other one
    /// switched keys), that is an error and never an overwrite.
    fn refresh(&self, secrets: &mut SecretVault, key: &str) -> Result<(), String> {
        let now = stamp_of(&self.path);
        if now == *self.stamp.lock().unwrap() {
            return Ok(());
        }
        match try_open(&self.path, key)? {
            Some(fresh) => {
                *secrets = fresh;
                *self.stamp.lock().unwrap() = now;
                Ok(())
            }
            None => Err("The saved passwords were changed by another BHTerminal window. \
                Quit BHTerminal from the tray icon and start it again."
                .into()),
        }
    }

    fn discard_staged(&self) {
        let _ = std::fs::remove_file(next_key_file(&self.data_dir));
        let _ = std::fs::remove_file(next_lock_file(&self.data_dir));
    }

    /// Makes the staged key the current one. The vault is ALREADY under it,
    /// so every step here only removes ways back to the old key. The plain
    /// key is either replaced by the staged one or removed — never left as
    /// the old key, which no longer opens anything.
    fn promote_staged(&self) -> io::Result<()> {
        let d = &self.data_dir;
        if next_lock_file(d).exists() {
            std::fs::rename(next_lock_file(d), lock_file(d))?;
        }
        if next_key_file(d).exists() {
            std::fs::rename(next_key_file(d), key_file(d))
        } else {
            remove_if_present(&key_file(d))
        }
    }

    /// The key behind the lock at `at`, if `master` is right.
    fn unwrap_at(at: &Path, master: &str) -> Result<String, String> {
        let bytes = std::fs::read(at).map_err(|e| io_err("vault lock", e))?;
        lock::unwrap(&bytes, master).map_err(|e| e.to_string())
    }

    /// Checks `master` against the lock AND the key in use — so a lock file
    /// swapped in from elsewhere cannot vouch for a password.
    fn check(&self, key: &str, master: &str) -> Result<(), String> {
        if Self::unwrap_at(&lock_file(&self.data_dir), master)? != key {
            return Err(LockError::WrongPassword.to_string());
        }
        Ok(())
    }

    /// Moves the vault to a NEW key, sealed under `master`, in an order where
    /// a crash at any point still leaves one way in:
    ///   1. stage the new lock (and plain key, if remembered) as `.next`;
    ///   2. save the vault under the new key — the commit point;
    ///   3. promote the staged files over the old ones.
    /// Before 2, `open` finds the old key still fits and drops the staging.
    /// After 2, it finds only the staged key fits and finishes the job.
    ///
    /// Why a new key at all: an old copy of `vault.key` — a Time Machine
    /// backup from before the master password — would otherwise still open
    /// every password saved since.
    fn rotate(&self, secrets: &SecretVault, current_key: &mut String, master: &str, remember: bool) -> Result<(), String> {
        let d = &self.data_dir;
        self.discard_staged();
        let key = new_key();
        let bytes = lock::wrap(&key, master).map_err(|e| e.to_string())?;
        if lock::unwrap(&bytes, master).ok().as_deref() != Some(key.as_str()) {
            return Err("the new lock did not verify — nothing was changed".into());
        }
        let staged = (|| -> io::Result<()> {
            write_private(&next_lock_file(d), &bytes)?;
            if remember {
                write_private(&next_key_file(d), key.as_bytes())?;
            }
            Ok(())
        })();
        if let Err(e) = staged {
            self.discard_staged();
            return Err(io_err("vault lock", e));
        }
        if let Err(e) = self.save_secrets(secrets, &key) {
            self.discard_staged();
            return Err(e);
        }
        // Committed: from here the new key is the only one that works.
        *current_key = key;
        self.promote_staged()
            .map_err(|e| io_err("saved, but tidying the old key failed (it is finished at the next start)", e))
    }

    /// With a master password set, taking saved passwords OUT of the
    /// computer needs it. Without one there is nothing to check.
    pub fn check_export(&self, master: Option<&str>) -> Result<(), String> {
        match &*self.state.lock().unwrap() {
            State::Locked => Err(LOCKED.into()),
            State::Unlocked { has_master: false, .. } => Ok(()),
            State::Unlocked { key, .. } => self.check(key, master.unwrap_or("")),
        }
    }

    pub fn unlock(&self, master: &str, remember: bool) -> Result<(), String> {
        let mut st = self.state.lock().unwrap();
        if matches!(*st, State::Unlocked { .. }) {
            return Ok(());
        }
        // Normally only `vault.lock`; the staged one too if a switch to a new
        // key was interrupted after the vault had already moved to it.
        let mut last = LockError::WrongPassword.to_string();
        for (at, staged) in [(lock_file(&self.data_dir), false), (next_lock_file(&self.data_dir), true)] {
            if !at.exists() {
                continue;
            }
            let key = match Self::unwrap_at(&at, master) {
                Ok(k) => k,
                Err(e) => {
                    last = e;
                    continue;
                }
            };
            // A lock with no vault beside it is refused rather than taken as
            // an empty vault: a lock swapped in by someone else would
            // otherwise unlock "successfully" and collect every new secret.
            if !self.path.exists() {
                return Err("The saved-passwords file is missing, so this lock cannot be trusted.".into());
            }
            let Some(secrets) = try_open(&self.path, &key)? else { continue };
            if staged {
                self.promote_staged().map_err(|e| io_err("vault lock", e))?;
            }
            if remember {
                write_private(&key_file(&self.data_dir), key.as_bytes()).map_err(|e| io_err("vault key", e))?;
            }
            *self.stamp.lock().unwrap() = stamp_of(&self.path);
            *st = State::Unlocked { secrets, key, has_master: true };
            return Ok(());
        }
        Err(last)
    }

    /// Sets a master password, or changes it (`current` required then).
    pub fn set_master(&self, current: Option<&str>, new: &str, remember: bool) -> Result<(), String> {
        lock::check_master(new).map_err(|e| e.to_string())?;
        let mut st = self.state.lock().unwrap();
        let State::Unlocked { secrets, key, has_master } = &mut *st else { return Err(LOCKED.into()) };
        if *has_master {
            self.check(key, current.unwrap_or(""))?;
        }
        self.refresh(secrets, key)?;
        let result = self.rotate(secrets, key, new, remember);
        // Once the vault has moved, a master password IS set, even if the
        // tidy-up after it reported an error.
        if lock_file(&self.data_dir).exists() || next_lock_file(&self.data_dir).exists() {
            *has_master = true;
        }
        self.mark_asked();
        result
    }

    /// Back to no master password. The plain key is written BEFORE the lock
    /// goes, so there is never a moment with no way in.
    pub fn remove_master(&self, current: &str) -> Result<(), String> {
        let mut st = self.state.lock().unwrap();
        let State::Unlocked { key, has_master, .. } = &mut *st else { return Err(LOCKED.into()) };
        if !*has_master {
            return Err("There is no master password to remove.".into());
        }
        self.check(key, current)?;
        write_private(&key_file(&self.data_dir), key.as_bytes()).map_err(|e| io_err("vault key", e))?;
        remove_if_present(&lock_file(&self.data_dir)).map_err(|e| io_err("vault lock", e))?;
        *has_master = false;
        self.mark_asked();
        Ok(())
    }

    /// Turns "Remember on this computer" on or off. Asks for the password
    /// either way: turning it ON is exactly what someone at an unlocked,
    /// unattended computer would want to do. Turning it OFF moves the vault
    /// to a new key, so a copy of the old plain key opens nothing new.
    pub fn set_remember(&self, current: &str, remember: bool) -> Result<(), String> {
        let mut st = self.state.lock().unwrap();
        let State::Unlocked { secrets, key, has_master } = &mut *st else { return Err(LOCKED.into()) };
        if !*has_master {
            return Err("There is no master password to remember.".into());
        }
        self.check(key, current)?;
        if remember {
            write_private(&key_file(&self.data_dir), key.as_bytes()).map_err(|e| io_err("vault key", e))
        } else {
            self.refresh(secrets, key)?;
            self.rotate(secrets, key, current, false)
        }
    }

    /// "Not now" on the one-time question.
    pub fn mark_asked(&self) {
        let _ = write_private(&asked_file(&self.data_dir), b"1");
    }

    /// The forgotten-password way out. Only while LOCKED — an unlocked user
    /// removes the password instead and keeps everything. The locked vault
    /// and its lock are MOVED aside, not deleted, in case the password comes
    /// back to them; then a fresh, empty vault takes their place. If any move
    /// fails, the ones already made are put back, so a half-done reset can
    /// never look like an empty vault. Returns the folder they went to.
    pub fn reset_forgotten(&self, stamp: &str) -> Result<PathBuf, String> {
        let mut st = self.state.lock().unwrap();
        if !matches!(*st, State::Locked) {
            return Err("Saved passwords are not locked — nothing to reset.".into());
        }
        let d = &self.data_dir;
        let aside = d.join(format!("locked-vault-{stamp}"));
        std::fs::create_dir_all(&aside).map_err(|e| io_err("reset", e))?;
        let names = ["vault.lock", "vault.lock.next", "vault.key", "vault.key.next", "secrets.json"];
        let mut moved: Vec<&str> = Vec::new();
        for name in names {
            let from = d.join(name);
            if !from.exists() {
                continue;
            }
            if let Err(e) = std::fs::rename(&from, aside.join(name)) {
                for back in moved.iter().rev() {
                    let _ = std::fs::rename(aside.join(back), d.join(back));
                }
                return Err(io_err("reset — nothing was changed", e));
            }
            moved.push(name);
        }
        let key = new_key();
        write_private(&key_file(d), key.as_bytes()).map_err(|e| io_err("vault key", e))?;
        // Ask again: having just lost one, they may want a new password.
        remove_if_present(&asked_file(d)).map_err(|e| io_err("reset", e))?;
        *self.stamp.lock().unwrap() = stamp_of(&self.path);
        *st = State::Unlocked { secrets: SecretVault::new(), key, has_master: false };
        Ok(aside)
    }

    pub fn get(&self, account: &str) -> Option<String> {
        match &mut *self.state.lock().unwrap() {
            State::Unlocked { secrets, key, .. } => {
                // Best effort: a failed refresh keeps what this process knows.
                let _ = self.refresh(secrets, key);
                secrets.get(account).map(|s| s.to_string())
            }
            State::Locked => None,
        }
    }

    pub fn set(&self, account: &str, secret: &str) -> Result<(), String> {
        self.mutate(|v| v.set(account, secret))
    }

    pub fn remove(&self, accounts: &[String]) -> Result<(), String> {
        let mut st = self.state.lock().unwrap();
        let State::Unlocked { secrets, key, .. } = &mut *st else { return Err(LOCKED.into()) };
        self.refresh(secrets, key)?;
        let mut changed = false;
        for a in accounts {
            changed |= secrets.remove(a);
        }
        if changed {
            self.save_secrets(secrets, key)?;
        }
        Ok(())
    }

    /// A copy for backup export. The locking belongs with the vault rather
    /// than with the backup code, which is why this lives here.
    pub fn snapshot(&self) -> Result<SecretVault, String> {
        match &mut *self.state.lock().unwrap() {
            State::Unlocked { secrets, key, .. } => {
                let _ = self.refresh(secrets, key);
                Ok(secrets.clone())
            }
            State::Locked => Err(LOCKED.into()),
        }
    }

    /// Applies a batch of changes under ONE lock and ONE save. An import can
    /// carry 700+ passwords; saving per secret would re-encrypt and rewrite
    /// the whole file that many times.
    pub fn mutate<T>(&self, f: impl FnOnce(&mut SecretVault) -> T) -> Result<T, String> {
        let mut st = self.state.lock().unwrap();
        let State::Unlocked { secrets, key, .. } = &mut *st else { return Err(LOCKED.into()) };
        self.refresh(secrets, key)?;
        let out = f(secrets);
        self.save_secrets(secrets, key)?;
        Ok(out)
    }
}

pub type SharedVault = Arc<Vault>;

// ── commands ───────────────────────────────────────────────────────────────
// Every one that derives a key runs on a blocking thread: the lock's PBKDF2
// is deliberately slow, and on the main thread it would freeze the window.

pub async fn blocking<T: Send + 'static>(
    vault: &SharedVault,
    f: impl FnOnce(&Vault) -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    let vault = vault.clone();
    tauri::async_runtime::spawn_blocking(move || f(&vault))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn vault_status(vault: tauri::State<'_, SharedVault>) -> VaultStatus {
    vault.status()
}

#[tauri::command]
pub async fn vault_unlock(
    vault: tauri::State<'_, SharedVault>,
    password: String,
    remember: bool,
) -> Result<(), String> {
    blocking(&vault, move |v| v.unlock(&password, remember)).await
}

/// Sets a master password, or changes it when `current` is given.
#[tauri::command]
pub async fn vault_set_master(
    vault: tauri::State<'_, SharedVault>,
    current: Option<String>,
    password: String,
    remember: bool,
) -> Result<(), String> {
    blocking(&vault, move |v| v.set_master(current.as_deref(), &password, remember)).await
}

#[tauri::command]
pub async fn vault_remove_master(
    vault: tauri::State<'_, SharedVault>,
    current: String,
) -> Result<(), String> {
    blocking(&vault, move |v| v.remove_master(&current)).await
}

#[tauri::command]
pub async fn vault_set_remember(
    vault: tauri::State<'_, SharedVault>,
    current: String,
    remember: bool,
) -> Result<(), String> {
    blocking(&vault, move |v| v.set_remember(&current, remember)).await
}

#[tauri::command]
pub fn vault_skip_setup(vault: tauri::State<'_, SharedVault>) {
    // Under the vault's lock, like every other write of the marker.
    let _guard = vault.state.lock().unwrap();
    vault.mark_asked();
}

/// Returns where the locked vault was moved, so the UI can say.
#[tauri::command]
pub fn vault_reset_forgotten(vault: tauri::State<'_, SharedVault>) -> Result<String, String> {
    let aside = vault.reset_forgotten(&crate::transfer::now_stamp())?;
    Ok(aside.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    fn reopen(d: &Path) -> Vault {
        Vault::open(d).unwrap()
    }

    fn plain_key(d: &Path) -> Option<String> {
        read_key_at(&key_file(d)).unwrap()
    }

    #[test]
    fn a_first_run_starts_unlocked_and_asks_the_question_once() {
        let d = dir();
        let v = reopen(d.path());
        let s = v.status();
        assert!(!s.locked && !s.has_master && s.ask_setup);
        v.mark_asked();
        assert!(!reopen(d.path()).status().ask_setup);
    }

    #[test]
    fn without_remember_the_next_start_is_locked_and_the_password_opens_it() {
        let d = dir();
        let v = reopen(d.path());
        v.set("host-1", "s3cret").unwrap();
        v.set_master(None, "correct horse", false).unwrap();
        assert!(!key_file(d.path()).exists(), "the plain key must be gone");

        let v = reopen(d.path());
        assert!(v.status().locked);
        assert_eq!(v.get("host-1"), None);
        assert!(v.set("host-2", "x").is_err(), "a locked vault must refuse writes");
        assert!(v.unlock("wrong password", false).is_err());
        v.unlock("correct horse", false).unwrap();
        assert_eq!(v.get("host-1").as_deref(), Some("s3cret"));
        // Unlocking without remember leaves nothing behind for next time.
        assert!(reopen(d.path()).status().locked);
    }

    #[test]
    fn a_master_password_set_on_an_empty_vault_still_locks_it() {
        // No secret saved yet: the vault file must still be written, or the
        // "lock without a vault" refusal would lock the user out.
        let d = dir();
        reopen(d.path()).set_master(None, "correct horse", false).unwrap();
        let v = reopen(d.path());
        v.unlock("correct horse", false).unwrap();
        v.set("host-1", "s3cret").unwrap();
    }

    #[test]
    fn setting_a_master_password_retires_the_old_plain_key() {
        // A copy of vault.key taken before the master password (a backup,
        // a synced folder) must not open passwords saved after it.
        let d = dir();
        let v = reopen(d.path());
        v.set("host-1", "old").unwrap();
        let old_key = plain_key(d.path()).unwrap();
        v.set_master(None, "correct horse", false).unwrap();
        v.set("host-2", "new").unwrap();
        let path = d.path().join("secrets.json");
        assert_eq!(try_open(&path, &old_key).unwrap(), None);
    }

    #[test]
    fn with_remember_the_next_start_is_silent() {
        let d = dir();
        let v = reopen(d.path());
        v.set("host-1", "s3cret").unwrap();
        v.set_master(None, "correct horse", true).unwrap();
        let v = reopen(d.path());
        let s = v.status();
        assert!(!s.locked && s.has_master && s.remembered);
        assert_eq!(v.get("host-1").as_deref(), Some("s3cret"));
    }

    #[test]
    fn ticking_remember_at_unlock_keeps_the_key() {
        let d = dir();
        let v = reopen(d.path());
        v.set_master(None, "correct horse", false).unwrap();
        reopen(d.path()).unlock("correct horse", true).unwrap();
        assert!(!reopen(d.path()).status().locked);
    }

    #[test]
    fn changing_needs_the_current_password_and_the_old_one_stops_working() {
        let d = dir();
        let v = reopen(d.path());
        v.set("host-1", "s3cret").unwrap();
        v.set_master(None, "correct horse", false).unwrap();
        assert!(v.set_master(Some("wrong password"), "battery staple", false).is_err());
        assert!(v.set_master(None, "battery staple", false).is_err());
        let old_lock = std::fs::read(lock_file(d.path())).unwrap();
        v.set_master(Some("correct horse"), "battery staple", false).unwrap();

        let v = reopen(d.path());
        assert!(v.unlock("correct horse", false).is_err());
        v.unlock("battery staple", false).unwrap();
        assert_eq!(v.get("host-1").as_deref(), Some("s3cret"));

        // A copy of the OLD lock and the OLD password open nothing current.
        let old_key = lock::unwrap(&old_lock, "correct horse").unwrap();
        assert_eq!(try_open(&d.path().join("secrets.json"), &old_key).unwrap(), None);
    }

    #[test]
    fn removing_the_master_password_goes_back_to_a_silent_unlock() {
        let d = dir();
        let v = reopen(d.path());
        v.set("host-1", "s3cret").unwrap();
        v.set_master(None, "correct horse", false).unwrap();
        assert!(v.remove_master("wrong password").is_err());
        assert!(lock_file(d.path()).exists(), "a failed remove must change nothing");
        v.remove_master("correct horse").unwrap();
        let v = reopen(d.path());
        let s = v.status();
        assert!(!s.locked && !s.has_master && !s.ask_setup);
        assert_eq!(v.get("host-1").as_deref(), Some("s3cret"));
    }

    #[test]
    fn remember_can_be_switched_only_with_the_password() {
        let d = dir();
        let v = reopen(d.path());
        v.set("host-1", "s3cret").unwrap();
        v.set_master(None, "correct horse", false).unwrap();
        assert!(v.set_remember("wrong password", true).is_err());
        assert!(!key_file(d.path()).exists());
        v.set_remember("correct horse", true).unwrap();
        let remembered = plain_key(d.path()).unwrap();
        assert!(!reopen(d.path()).status().locked);

        v.set_remember("correct horse", false).unwrap();
        assert!(reopen(d.path()).status().locked);
        // Forgetting moves to a new key: the remembered copy is now useless.
        assert_eq!(try_open(&d.path().join("secrets.json"), &remembered).unwrap(), None);
        let v = reopen(d.path());
        v.unlock("correct horse", false).unwrap();
        assert_eq!(v.get("host-1").as_deref(), Some("s3cret"));
    }

    #[test]
    fn the_checks_refuse_a_lock_that_does_not_match_the_key_in_use() {
        // A lock file copied in from another install, sealed under a password
        // the attacker knows, must not vouch for that password here.
        let d = dir();
        let v = reopen(d.path());
        v.set_master(None, "correct horse", true).unwrap();
        let other = lock::wrap(&new_key(), "attacker pass").unwrap();
        std::fs::write(lock_file(d.path()), other).unwrap();
        assert!(v.check_export(Some("attacker pass")).is_err());
        assert!(v.set_remember("attacker pass", true).is_err());
        assert!(v.remove_master("attacker pass").is_err());
    }

    #[test]
    fn deleting_the_lock_while_open_does_not_turn_the_export_check_off() {
        let d = dir();
        let v = reopen(d.path());
        v.set_master(None, "correct horse", true).unwrap();
        std::fs::remove_file(lock_file(d.path())).unwrap();
        assert!(v.check_export(None).is_err());
        assert!(v.check_export(Some("anything at all")).is_err());
    }

    #[test]
    fn a_lock_with_no_vault_beside_it_is_refused() {
        let d = dir();
        reopen(d.path()).set_master(None, "correct horse", false).unwrap();
        std::fs::remove_file(d.path().join("secrets.json")).unwrap();
        assert!(reopen(d.path()).unlock("correct horse", false).is_err());
    }

    #[test]
    fn an_interrupted_switch_before_the_vault_moved_keeps_the_old_key() {
        // Staged files written, crash before the vault was saved under them.
        let d = dir();
        let v = reopen(d.path());
        v.set("host-1", "s3cret").unwrap();
        let staged = new_key();
        std::fs::write(next_lock_file(d.path()), lock::wrap(&staged, "correct horse").unwrap()).unwrap();
        std::fs::write(next_key_file(d.path()), &staged).unwrap();
        drop(v);

        let v = reopen(d.path());
        assert_eq!(v.get("host-1").as_deref(), Some("s3cret"));
        assert!(!v.status().has_master);
        assert!(!next_lock_file(d.path()).exists() && !next_key_file(d.path()).exists());
    }

    #[test]
    fn an_interrupted_switch_after_the_vault_moved_finishes_at_the_next_start() {
        // Remembered: the staged plain key is the way in.
        let d = dir();
        let v = reopen(d.path());
        v.set("host-1", "s3cret").unwrap();
        let staged = new_key();
        std::fs::write(next_lock_file(d.path()), lock::wrap(&staged, "correct horse").unwrap()).unwrap();
        std::fs::write(next_key_file(d.path()), &staged).unwrap();
        v.snapshot().unwrap().save(&d.path().join("secrets.json"), &staged).unwrap();
        drop(v);

        let v = reopen(d.path());
        assert_eq!(v.get("host-1").as_deref(), Some("s3cret"));
        assert!(v.status().has_master);
        assert_eq!(plain_key(d.path()).as_deref(), Some(staged.as_str()));
        assert!(!next_lock_file(d.path()).exists());
    }

    #[test]
    fn an_interrupted_unremembered_switch_asks_and_the_new_password_opens_it() {
        // Not remembered: only the staged LOCK holds the new key, and the old
        // plain key no longer fits. The app must ask, not fail to start.
        let d = dir();
        let v = reopen(d.path());
        v.set("host-1", "s3cret").unwrap();
        let staged = new_key();
        std::fs::write(next_lock_file(d.path()), lock::wrap(&staged, "correct horse").unwrap()).unwrap();
        v.snapshot().unwrap().save(&d.path().join("secrets.json"), &staged).unwrap();
        drop(v);

        let v = reopen(d.path());
        assert!(v.status().locked);
        v.unlock("correct horse", false).unwrap();
        assert_eq!(v.get("host-1").as_deref(), Some("s3cret"));
        assert!(lock_file(d.path()).exists() && !next_lock_file(d.path()).exists());
        assert!(!key_file(d.path()).exists(), "the stale plain key must be gone");
    }

    #[test]
    fn forgetting_the_password_moves_the_old_vault_aside_and_starts_empty() {
        let d = dir();
        let v = reopen(d.path());
        v.set("host-1", "s3cret").unwrap();
        v.set_master(None, "correct horse", false).unwrap();
        assert!(v.reset_forgotten("x").is_err(), "an unlocked vault is never reset");

        let v = reopen(d.path());
        let aside = v.reset_forgotten("2026-10-13").unwrap();
        assert!(aside.join("secrets.json").exists() && aside.join("vault.lock").exists());
        let s = v.status();
        assert!(!s.locked && !s.has_master && s.ask_setup);
        assert_eq!(v.get("host-1"), None);
        v.set("host-1", "new").unwrap();
        assert_eq!(reopen(d.path()).get("host-1").as_deref(), Some("new"));

        // The moved-aside vault still opens with the old password.
        let bytes = std::fs::read(aside.join("vault.lock")).unwrap();
        let old_key = lock::unwrap(&bytes, "correct horse").unwrap();
        let old = SecretVault::load(&aside.join("secrets.json"), &old_key).unwrap();
        assert_eq!(old.get("host-1"), Some("s3cret"));
    }

    #[test]
    fn a_vault_without_any_key_is_an_error_not_an_empty_start() {
        let d = dir();
        let v = reopen(d.path());
        v.set("host-1", "s3cret").unwrap();
        std::fs::remove_file(key_file(d.path())).unwrap();
        assert!(Vault::open(d.path()).is_err());
        assert!(d.path().join("secrets.json").exists());
    }

    #[test]
    fn two_windows_on_one_data_folder_do_not_erase_each_other() {
        // The Windows report: a copy started earlier holds an old picture of
        // the vault. A password saved in the OTHER copy must be visible to
        // it, and its own next save must keep that password.
        let d = dir();
        let early = reopen(d.path());
        let late = reopen(d.path());
        late.set("host-A", "from-the-second-window").unwrap();
        // The first window never saved anything, but it sees A…
        assert_eq!(early.get("host-A").as_deref(), Some("from-the-second-window"));
        // …and saving B in the first window keeps A on disk.
        early.set("host-B", "from-the-first-window").unwrap();
        let fresh = reopen(d.path());
        assert_eq!(fresh.get("host-A").as_deref(), Some("from-the-second-window"));
        assert_eq!(fresh.get("host-B").as_deref(), Some("from-the-first-window"));
    }

    #[test]
    fn a_stale_window_refuses_to_overwrite_after_the_other_switched_keys() {
        // The other window set a master password (a NEW key). This one's key
        // no longer opens the file: it must say so, never overwrite.
        let d = dir();
        let stale = reopen(d.path());
        stale.set("host-A", "one").unwrap();
        let other = reopen(d.path());
        other.set_master(None, "correct horse", false).unwrap();
        other.set("host-B", "two").unwrap();
        assert!(stale.set("host-C", "three").is_err());
        // Nothing was overwritten: the other window's data is intact.
        let after = reopen(d.path());
        after.unlock("correct horse", false).unwrap();
        assert_eq!(after.get("host-B").as_deref(), Some("two"));
        assert_eq!(after.get("host-A").as_deref(), Some("one"));
    }

    #[test]
    fn a_leftover_temp_key_is_removed_at_start() {
        let d = dir();
        drop(reopen(d.path()));
        std::fs::write(tmp_for(&key_file(d.path())), "plain key from a crash").unwrap();
        drop(reopen(d.path()));
        assert!(!tmp_for(&key_file(d.path())).exists());
    }

    #[cfg(unix)]
    #[test]
    fn key_and_lock_files_are_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let d = dir();
        let v = reopen(d.path());
        v.set_master(None, "correct horse", true).unwrap();
        for f in [key_file(d.path()), lock_file(d.path()), d.path().join("secrets.json")] {
            let mode = std::fs::metadata(&f).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "{}", f.display());
        }
    }
}
