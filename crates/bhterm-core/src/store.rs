//! The saved-session document: folders, hosts and snippets in one JSON file.
//!
//! Wire-compatible with the shipped macOS app's `sessions.json` (see model.rs).
//! The shell supplies the path — the core never discovers it.

use crate::model::{build_tree, Host, HostFolder, Snippet, TreeItem};
use serde::{Deserialize, Serialize};
use std::path::Path;
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("Couldn't read or write the session file: {0}")]
    Io(String),
    #[error("The session file could not be parsed: {0}")]
    Parse(String),
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionStore {
    #[serde(default)]
    pub folders: Vec<HostFolder>,
    #[serde(default)]
    pub hosts: Vec<Host>,
    #[serde(default)]
    pub snippets: Vec<Snippet>,
}

impl SessionStore {
    /// A missing file is a first run, not a failure. A file that exists but
    /// cannot be parsed IS a failure and must be reported — silently starting
    /// empty would invite overwriting someone's whole host list on the next
    /// save.
    pub fn load(path: &Path) -> Result<Self, StoreError> {
        let raw = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(e) => return Err(StoreError::Io(e.to_string())),
        };
        serde_json::from_slice(&raw).map_err(|e| StoreError::Parse(e.to_string()))
    }

    pub fn save(&self, path: &Path) -> Result<(), StoreError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| StoreError::Io(e.to_string()))?;
        }
        let json = serde_json::to_vec_pretty(self).map_err(|e| StoreError::Parse(e.to_string()))?;
        // Write-then-rename: a crash mid-write must not truncate the file.
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, &json).map_err(|e| StoreError::Io(e.to_string()))?;
        std::fs::rename(&tmp, path).map_err(|e| StoreError::Io(e.to_string()))?;
        Ok(())
    }

    pub fn tree(&self) -> Vec<TreeItem> {
        build_tree(&self.folders, &self.hosts, None)
    }

    pub fn host(&self, id: Uuid) -> Option<&Host> {
        self.hosts.iter().find(|h| h.id == id)
    }

    /// Appends, giving the host the next sort position within its folder.
    pub fn add_host(&mut self, mut host: Host) -> Host {
        host.sort_order = self
            .hosts
            .iter()
            .filter(|h| h.folder_id == host.folder_id)
            .map(|h| h.sort_order)
            .max()
            .map_or(0, |m| m + 1);
        self.hosts.push(host.clone());
        host
    }

    pub fn update_host(&mut self, host: Host) -> bool {
        match self.hosts.iter_mut().find(|h| h.id == host.id) {
            Some(slot) => {
                *slot = host;
                true
            }
            None => false,
        }
    }

    /// Returns the secret accounts the caller should now purge from the vault
    /// — the store has no business reaching into credentials itself.
    pub fn delete_host(&mut self, id: Uuid) -> Vec<String> {
        self.hosts.retain(|h| h.id != id);
        vec![Host::secret_account(id), Host::passphrase_account(id)]
    }

    pub fn add_folder(&mut self, mut folder: HostFolder) -> HostFolder {
        folder.sort_order = self
            .folders
            .iter()
            .filter(|f| f.parent_folder_id == folder.parent_folder_id)
            .map(|f| f.sort_order)
            .max()
            .map_or(0, |m| m + 1);
        self.folders.push(folder.clone());
        folder
    }

    pub fn update_folder(&mut self, folder: HostFolder) -> bool {
        match self.folders.iter_mut().find(|f| f.id == folder.id) {
            Some(slot) => {
                *slot = folder;
                true
            }
            None => false,
        }
    }

    /// Cascades into subfolders and their hosts, returning every secret
    /// account freed by the deletion.
    pub fn delete_folder(&mut self, id: Uuid) -> Vec<String> {
        let mut doomed_folders = vec![id];
        let mut i = 0;
        while i < doomed_folders.len() {
            let parent = doomed_folders[i];
            let children: Vec<Uuid> = self
                .folders
                .iter()
                .filter(|f| f.parent_folder_id == Some(parent))
                .map(|f| f.id)
                .collect();
            doomed_folders.extend(children);
            i += 1;
        }

        let mut freed = Vec::new();
        for host in self
            .hosts
            .iter()
            .filter(|h| h.folder_id.is_some_and(|f| doomed_folders.contains(&f)))
        {
            freed.push(Host::secret_account(host.id));
            freed.push(Host::passphrase_account(host.id));
        }
        self.hosts
            .retain(|h| !h.folder_id.is_some_and(|f| doomed_folders.contains(&f)));
        self.folders.retain(|f| !doomed_folders.contains(&f.id));
        freed
    }

    pub fn add_snippet(&mut self, mut snippet: Snippet) -> Snippet {
        snippet.sort_order = self.snippets.iter().map(|s| s.sort_order).max().map_or(0, |m| m + 1);
        self.snippets.push(snippet.clone());
        snippet
    }

    pub fn update_snippet(&mut self, snippet: Snippet) -> bool {
        match self.snippets.iter_mut().find(|s| s.id == snippet.id) {
            Some(slot) => {
                *slot = snippet;
                true
            }
            None => false,
        }
    }

    pub fn delete_snippet(&mut self, id: Uuid) {
        self.snippets.retain(|s| s.id != id);
    }
}
