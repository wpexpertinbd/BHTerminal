//! The saved-session data model.
//!
//! ⚠️ The JSON here is **wire-compatible with the shipped Swift app's
//! `sessions.json`** — this build replaces that app on machines that already
//! have one, so a mismatch silently loses someone's whole host list.
//! Swift's `Codable` encodes an enum case as an object keyed by the case name
//! (`{"password":{}}`, `{"privateKey":{"path":"…"}}`), which is why the unit
//! cases below are written as empty struct variants: serde renders those as
//! `{"password":{}}` too, whereas a plain unit variant would render as the
//! bare string `"password"` and fail to load.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use uuid::Uuid;

/// Swift writes UUIDs uppercase. Both sides parse either case, but emitting
/// uppercase keeps a file written here byte-identical to one written there,
/// which matters while both apps can still open the same profile.
pub mod uuid_upper {
    use super::*;

    pub fn serialize<S: Serializer>(id: &Uuid, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&id.to_string().to_uppercase())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Uuid, D::Error> {
        let raw = String::deserialize(d)?;
        raw.parse().map_err(serde::de::Error::custom)
    }

    pub mod option {
        use super::*;

        pub fn serialize<S: Serializer>(id: &Option<Uuid>, s: S) -> Result<S::Ok, S::Error> {
            match id {
                Some(id) => s.serialize_str(&id.to_string().to_uppercase()),
                None => s.serialize_none(),
            }
        }

        pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Uuid>, D::Error> {
            let raw = Option::<String>::deserialize(d)?;
            match raw {
                Some(s) => s.parse().map(Some).map_err(serde::de::Error::custom),
                None => Ok(None),
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AuthMethod {
    /// Empty struct variants, not unit variants — see the module note.
    Password {},
    PrivateKey { path: String },
    Agent {},
}

impl Default for AuthMethod {
    fn default() -> Self {
        AuthMethod::Agent {}
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ConnectionType {
    #[default]
    Ssh,
    Vnc,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TunnelKind {
    /// ssh -L listenPort:destHost:destPort
    Local,
    /// ssh -R listenPort:destHost:destPort
    Remote,
    /// ssh -D listenPort (SOCKS proxy)
    Dynamic,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TunnelRule {
    #[serde(with = "uuid_upper", default = "new_uuid")]
    pub id: Uuid,
    #[serde(default)]
    pub name: String,
    pub kind: TunnelKind,
    #[serde(default = "loopback")]
    pub listen_host: String,
    pub listen_port: u16,
    #[serde(default)]
    pub dest_host: String,
    #[serde(default)]
    pub dest_port: u16,
    #[serde(default = "yes")]
    pub enabled: bool,
}

fn new_uuid() -> Uuid {
    Uuid::new_v4()
}
fn loopback() -> String {
    "127.0.0.1".into()
}
fn yes() -> bool {
    true
}
fn default_port() -> u16 {
    22
}

/// A saved connection. Never carries a secret: passwords and key passphrases
/// live in the vault, referenced by the accounts derived below, so this file
/// stays safe to copy or sync.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Host {
    #[serde(with = "uuid_upper", default = "new_uuid")]
    pub id: Uuid,
    pub name: String,
    pub hostname: String,
    #[serde(default = "default_port")]
    pub port: u16,
    pub username: String,
    #[serde(default)]
    pub auth_method: AuthMethod,
    // Added after hosts were already being saved, so it must tolerate its
    // absence — one missing key must never drop the whole document.
    #[serde(default)]
    pub connection_type: ConnectionType,
    // ⚠️ Swift spells these "…ID", which rename_all = "camelCase" would turn
    // into "…Id" — silently dropping every host's folder on load. Pinned.
    #[serde(rename = "jumpHostID", with = "uuid_upper::option", default)]
    pub jump_host_id: Option<Uuid>,
    #[serde(rename = "folderID", with = "uuid_upper::option", default)]
    pub folder_id: Option<Uuid>,
    #[serde(default)]
    pub color_tag: Option<String>,
    #[serde(default)]
    pub tunnels: Vec<TunnelRule>,
    #[serde(default)]
    pub sort_order: i32,
    #[serde(default)]
    pub notes: String,
    /// VNC only, opt-in: a malicious or MITM'd server can otherwise read and
    /// poison the local clipboard.
    #[serde(default)]
    pub vnc_shared_clipboard: bool,
}

impl Host {
    pub fn new(name: impl Into<String>, hostname: impl Into<String>, username: impl Into<String>) -> Self {
        Host {
            id: Uuid::new_v4(),
            name: name.into(),
            hostname: hostname.into(),
            port: 22,
            username: username.into(),
            auth_method: AuthMethod::default(),
            connection_type: ConnectionType::Ssh,
            jump_host_id: None,
            folder_id: None,
            color_tag: None,
            tunnels: Vec::new(),
            sort_order: 0,
            notes: String::new(),
            vnc_shared_clipboard: false,
        }
    }

    pub fn secret_account(id: Uuid) -> String {
        format!("host.{}", id.to_string().to_uppercase())
    }

    pub fn passphrase_account(id: Uuid) -> String {
        format!("host.{}.passphrase", id.to_string().to_uppercase())
    }

    pub fn secret_account_for(&self) -> String {
        Self::secret_account(self.id)
    }

    pub fn passphrase_account_for(&self) -> String {
        Self::passphrase_account(self.id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostFolder {
    #[serde(with = "uuid_upper", default = "new_uuid")]
    pub id: Uuid,
    pub name: String,
    #[serde(rename = "parentFolderID", with = "uuid_upper::option", default)]
    pub parent_folder_id: Option<Uuid>,
    #[serde(default)]
    pub sort_order: i32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Snippet {
    #[serde(with = "uuid_upper", default = "new_uuid")]
    pub id: Uuid,
    pub name: String,
    pub command: String,
    #[serde(default)]
    pub sort_order: i32,
}

/// Read-only tree over the flat folder/host lists, built on demand for the
/// sidebar. Folders and hosts stay flat on disk — simple to persist.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum TreeItem {
    Folder {
        folder: HostFolder,
        children: Vec<TreeItem>,
    },
    Host {
        host: Host,
    },
}

impl TreeItem {
    pub fn id(&self) -> Uuid {
        match self {
            TreeItem::Folder { folder, .. } => folder.id,
            TreeItem::Host { host } => host.id,
        }
    }
}

pub fn build_tree(folders: &[HostFolder], hosts: &[Host], parent: Option<Uuid>) -> Vec<TreeItem> {
    let mut child_folders: Vec<&HostFolder> =
        folders.iter().filter(|f| f.parent_folder_id == parent).collect();
    child_folders.sort_by_key(|f| f.sort_order);

    let mut child_hosts: Vec<&Host> = hosts.iter().filter(|h| h.folder_id == parent).collect();
    child_hosts.sort_by_key(|h| h.sort_order);

    let mut out: Vec<TreeItem> = child_folders
        .into_iter()
        .map(|f| TreeItem::Folder {
            folder: f.clone(),
            children: build_tree(folders, hosts, Some(f.id)),
        })
        .collect();
    out.extend(child_hosts.into_iter().map(|h| TreeItem::Host { host: h.clone() }));
    out
}
