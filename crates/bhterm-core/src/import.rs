//! Importing sessions from other tools.
//!
//! Three formats, all plain text, all of them untrusted input: a MobaXterm
//! session export, MobaXterm's stored-passwords dump, and an OpenSSH client
//! config. Parsing is pure — nothing here touches the filesystem — so the
//! awkward real-world files can be unit-tested directly.
//!
//! An import file is exactly the "attacker-supplied host config" case, so
//! names are clamped and stripped of control characters and every hostname
//! and username goes back through the ssh argv guard before it is stored.

use crate::model::{AuthMethod, ConnectionType, Host, HostFolder};
use crate::secrets::SecretVault;
use crate::ssh;
use crate::store::SessionStore;
use std::collections::HashMap;

/// One host read out of an export, before anything is committed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedSession {
    pub name: String,
    pub hostname: String,
    pub port: u16,
    pub username: String,
    pub connection_type: ConnectionType,
    /// Nested folder names, outermost first; empty = directly under the
    /// import's own root folder.
    pub folder_path: Vec<String>,
    /// Only the stored-passwords dump carries one. It goes to the vault, never
    /// to sessions.json.
    pub password: Option<String>,
}

impl ParsedSession {
    fn ssh(name: String, hostname: String, port: u16, username: String, folder_path: Vec<String>) -> Self {
        ParsedSession {
            name,
            hostname,
            port,
            username,
            connection_type: ConnectionType::Ssh,
            folder_path,
            password: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    MobaXterm,
    MobaXtermPasswords,
    SshConfig,
}

impl Source {
    pub fn label(self) -> &'static str {
        match self {
            Source::MobaXterm => "MobaXterm",
            Source::MobaXtermPasswords => "MobaXterm Passwords",
            Source::SshConfig => "SSH Config",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportError {
    /// A MobaXterm config with no readable `[Bookmarks]` section: the file is
    /// sealed with the user's master password. Worth its own error because the
    /// way out is an export setting, not anything we can fix here.
    EncryptedMobaConf,
    UnrecognizedFormat,
}

impl std::fmt::Display for ImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ImportError::EncryptedMobaConf => f.write_str(
                "This MobaXterm configuration is encrypted with your master password, so its \
                 contents cannot be read. In MobaXterm, right-click “User sessions” → “Export \
                 sessions (.mxtsessions)”, or export your stored passwords from Settings → \
                 Configuration → General → “passwords management” → “Export to file”, and import \
                 that instead.",
            ),
            ImportError::UnrecognizedFormat => f.write_str(
                "Could not recognise this file. Supported: MobaXterm sessions (.mxtsessions, a \
                 plain .mobaconf or MobaXterm.ini), a MobaXterm stored-passwords export, and an \
                 OpenSSH config file.",
            ),
        }
    }
}

impl std::error::Error for ImportError {}

#[derive(Debug)]
pub struct Parsed {
    pub source: Source,
    pub sessions: Vec<ParsedSession>,
    pub warnings: Vec<String>,
}

/// Picks the format from the contents, not the file extension — people rename
/// these files, and the extension is wrong more often than the content is.
pub fn parse(contents: &str, filename: &str) -> Result<Parsed, ImportError> {
    if contents.contains("[Bookmarks") {
        let (sessions, warnings) = parse_mobaxterm(contents);
        return Ok(Parsed { source: Source::MobaXterm, sessions, warnings });
    }
    if looks_like_moba_passwords(contents) {
        let (sessions, warnings) = parse_moba_passwords(contents);
        return Ok(Parsed { source: Source::MobaXtermPasswords, sessions, warnings });
    }

    let ext = filename
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default();
    if ["mobaconf", "mxtsessions", "ini"].contains(&ext.as_str()) {
        return Err(ImportError::EncryptedMobaConf);
    }

    if looks_like_ssh_config(contents) {
        let (sessions, warnings) = parse_ssh_config(contents);
        return Ok(Parsed { source: Source::SshConfig, sessions, warnings });
    }

    Err(ImportError::UnrecognizedFormat)
}

fn looks_like_ssh_config(contents: &str) -> bool {
    contents
        .lines()
        .any(|l| l.trim_start().to_ascii_lowercase().starts_with("host "))
}

/// Three `user@host = …` lines is enough to be sure; fewer could be any
/// key/value file that happens to mention an email address.
fn looks_like_moba_passwords(contents: &str) -> bool {
    contents
        .lines()
        .filter(|l| {
            l.trim()
                .split_once(" = ")
                .is_some_and(|(key, _)| key.contains('@'))
        })
        .count()
        >= 3
}

// ── MobaXterm sessions ─────────────────────────────────────────────────────

/// `[Bookmarks*]` sections, one folder each (named by `SubRep`), holding
/// `name=#<type>#<flags>%host%port%user%…` lines. Type 109 is SSH and 97 is
/// VNC; everything else is reported rather than dropped in silence. This
/// format never contains passwords.
pub fn parse_mobaxterm(contents: &str) -> (Vec<ParsedSession>, Vec<String>) {
    let mut sessions = Vec::new();
    let mut warnings = Vec::new();
    let mut in_bookmarks = false;
    let mut sub_rep = String::new();

    for raw in contents.lines() {
        let line = raw.trim();
        if line.starts_with('[') && line.ends_with(']') {
            in_bookmarks = line[1..line.len() - 1].starts_with("Bookmarks");
            sub_rep.clear();
            continue;
        }
        if !in_bookmarks || line.is_empty() {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else { continue };
        let key = key.trim();
        if key == "SubRep" {
            sub_rep = value.trim().to_string();
            continue;
        }
        if key == "ImgNum" {
            continue;
        }

        let value = value.trim();
        let Some(after_hash) = value.strip_prefix('#') else { continue };
        let Some((kind, rest)) = after_hash.split_once('#') else { continue };
        let Ok(kind) = kind.parse::<u32>() else { continue };

        let fields: Vec<&str> = rest.split('%').collect();
        let field = |i: usize| fields.get(i).copied().unwrap_or_default();
        let folder_path: Vec<String> = sub_rep
            .split('\\')
            .filter(|c| !c.is_empty())
            .map(str::to_string)
            .collect();

        match kind {
            109 => sessions.push(ParsedSession::ssh(
                key.to_string(),
                field(1).to_string(),
                field(2).parse().unwrap_or(22),
                field(3).to_string(),
                folder_path,
            )),
            97 => sessions.push(ParsedSession {
                name: key.to_string(),
                hostname: field(1).to_string(),
                port: field(2).parse().unwrap_or(5900),
                username: String::new(),
                connection_type: ConnectionType::Vnc,
                folder_path,
                password: None,
            }),
            other => warnings.push(format!(
                "Skipped “{key}” — MobaXterm session type {other} is not supported (only SSH and VNC are imported)."
            )),
        }
    }

    (sessions, warnings)
}

// ── MobaXterm stored passwords ─────────────────────────────────────────────

struct TypedKey {
    /// None = a protocol we recognised but do not support (rdp, telnet, …).
    kind: Option<ConnectionType>,
    port: u16,
    user: String,
    host: String,
}

/// `user@host`, split on the LAST `@`: a username may contain one, a hostname
/// never does.
fn parse_bare_key(key: &str) -> Option<(String, String)> {
    let at = key.rfind('@')?;
    let (user, host) = (&key[..at], &key[at + 1..]);
    (!user.is_empty() && !host.is_empty()).then(|| (user.to_string(), host.to_string()))
}

/// `proto[port]:user@host`. None when there is no `proto:` prefix at all.
fn parse_typed_key(key: &str) -> Option<TypedKey> {
    let (prefix, rest) = key.split_once(':')?;
    let letters: String = prefix.chars().take_while(|c| c.is_alphabetic()).collect();
    let digits = &prefix[letters.len()..];
    if letters.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let (user, host) = parse_bare_key(rest)?;
    let port: Option<u16> = digits.parse().ok();

    Some(match letters.to_ascii_lowercase().as_str() {
        "ssh" | "sftp" => TypedKey { kind: Some(ConnectionType::Ssh), port: port.unwrap_or(22), user, host },
        "vnc" => TypedKey { kind: Some(ConnectionType::Vnc), port: port.unwrap_or(5900), user, host },
        _ => TypedKey { kind: None, port: port.unwrap_or(0), user, host },
    })
}

/// `[proto[port]:]user@host = password`, one per line. Typed entries win over
/// bare `user@host` ones, which are the same credential written again without
/// a protocol — so a bare line only creates a host when nothing typed covers
/// that user and host already.
pub fn parse_moba_passwords(contents: &str) -> (Vec<ParsedSession>, Vec<String>) {
    let mut by_key: HashMap<String, ParsedSession> = HashMap::new();
    let mut order: Vec<String> = Vec::new();
    let mut unsupported = 0usize;

    let pairs: Vec<(&str, &str)> = contents
        .lines()
        .filter_map(|l| l.trim().split_once(" = "))
        .map(|(k, v)| (k.trim(), v))
        .filter(|(k, _)| !k.is_empty())
        .collect();

    for (key, password) in &pairs {
        let Some(typed) = parse_typed_key(key) else { continue };
        let Some(kind) = typed.kind else {
            unsupported += 1;
            continue;
        };
        let map_key = format!("{kind:?}|{}|{}", typed.host, typed.user);
        order.push(map_key.clone());
        by_key.entry(map_key).or_insert(ParsedSession {
            name: format!("{}@{}", typed.user, typed.host),
            hostname: typed.host,
            port: typed.port,
            username: typed.user,
            connection_type: kind,
            folder_path: Vec::new(),
            password: (!password.is_empty()).then(|| password.to_string()),
        });
    }

    for (key, password) in &pairs {
        if parse_typed_key(key).is_some() {
            continue;
        }
        let Some((user, host)) = parse_bare_key(key) else { continue };
        let map_key = format!("{:?}|{host}|{user}", ConnectionType::Ssh);
        order.push(map_key.clone());
        by_key.entry(map_key).or_insert(ParsedSession {
            name: format!("{user}@{host}"),
            hostname: host,
            port: 22,
            username: user,
            connection_type: ConnectionType::Ssh,
            folder_path: Vec::new(),
            password: (!password.is_empty()).then(|| password.to_string()),
        });
    }

    let mut warnings = Vec::new();
    if unsupported > 0 {
        warnings.push(format!(
            "Skipped {unsupported} entr{} for protocols BHTerminal does not support (only SSH and VNC are imported).",
            if unsupported == 1 { "y" } else { "ies" }
        ));
    }

    // `order` can name the same key twice; keep the first mention's position.
    let mut seen = Vec::new();
    let sessions = order
        .into_iter()
        .filter(|k| {
            let first = !seen.contains(k);
            if first {
                seen.push(k.clone());
            }
            first
        })
        .filter_map(|k| by_key.remove(&k))
        .collect();
    (sessions, warnings)
}

// ── OpenSSH config ─────────────────────────────────────────────────────────

/// `Host` / `HostName` / `Port` / `User`. Wildcard patterns are rules rather
/// than connectable hosts, so they are skipped.
pub fn parse_ssh_config(contents: &str) -> (Vec<ParsedSession>, Vec<String>) {
    let mut sessions = Vec::new();
    let mut aliases: Vec<String> = Vec::new();
    let mut host_name = String::new();
    let mut user = String::new();
    let mut port: u16 = 22;

    fn flush(
        aliases: &[String],
        host_name: &str,
        user: &str,
        port: u16,
        out: &mut Vec<ParsedSession>,
    ) {
        for alias in aliases.iter().filter(|a| !a.contains('*') && !a.contains('?')) {
            out.push(ParsedSession::ssh(
                alias.clone(),
                if host_name.is_empty() { alias.clone() } else { host_name.to_string() },
                port,
                user.to_string(),
                Vec::new(),
            ));
        }
    }

    for raw in contents.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((keyword, value)) = split_config_line(line) else { continue };
        match keyword.to_ascii_lowercase().as_str() {
            "host" => {
                flush(&aliases, &host_name, &user, port, &mut sessions);
                aliases = value.split_whitespace().map(str::to_string).collect();
                host_name.clear();
                user.clear();
                port = 22;
            }
            "hostname" => host_name = value.to_string(),
            "user" => user = value.to_string(),
            "port" => port = value.parse().unwrap_or(22),
            _ => {}
        }
    }
    flush(&aliases, &host_name, &user, port, &mut sessions);

    (sessions, Vec::new())
}

/// ssh_config accepts whitespace OR `=` between keyword and value.
fn split_config_line(line: &str) -> Option<(&str, &str)> {
    let sep = line.find(|c: char| c == ' ' || c == '\t' || c == '=')?;
    let keyword = &line[..sep];
    let value = line[sep + 1..].trim_matches(|c: char| c == ' ' || c == '\t' || c == '=');
    (!keyword.is_empty() && !value.is_empty()).then_some((keyword, value))
}

// ── Committing ─────────────────────────────────────────────────────────────

#[derive(Debug, Default, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportResult {
    pub source: String,
    pub hosts_added: usize,
    pub folders_added: usize,
    pub skipped: usize,
    pub passwords_saved: usize,
    pub warnings: Vec<String>,
}

/// Import names come from an untrusted file: strip control characters (a
/// newline would corrupt the sidebar and the stored JSON alike) and clamp the
/// length so a hostile export cannot write a megabyte-long name.
fn sanitised_name(raw: &str) -> String {
    raw.chars()
        .filter(|c| !c.is_control())
        .collect::<String>()
        .trim()
        .chars()
        .take(200)
        .collect()
}

/// Everything lands under one folder named for the source, with the export's
/// own nesting recreated beneath it, so an import can be reviewed — or
/// deleted — as a unit instead of being scattered through the tree.
///
/// `default_username` is used when an export records no username, which the
/// ssh config format allows; the core does not know who is logged in.
pub fn commit(
    source: Source,
    sessions: &[ParsedSession],
    warnings: Vec<String>,
    store: &mut SessionStore,
    vault: Option<&mut SecretVault>,
    default_username: &str,
) -> ImportResult {
    let mut result = ImportResult { source: source.label().to_string(), warnings, ..Default::default() };
    if sessions.is_empty() {
        result.warnings.push("No sessions were found to import.".into());
        return result;
    }

    let root = store.add_folder(HostFolder {
        id: uuid::Uuid::new_v4(),
        name: format!("{} Import", source.label()),
        parent_folder_id: None,
        sort_order: 0,
    });
    result.folders_added += 1;

    let mut folder_cache: HashMap<String, uuid::Uuid> = HashMap::new();
    let mut vault = vault;

    for session in sessions {
        if !ssh::is_valid_hostname(&session.hostname) {
            result.skipped += 1;
            result.warnings.push(format!(
                "Skipped “{}” — “{}” is not a usable hostname.",
                sanitised_name(&session.name),
                sanitised_name(&session.hostname)
            ));
            continue;
        }

        let mut username = session.username.clone();
        if session.connection_type == ConnectionType::Ssh {
            if username.is_empty() {
                username = default_username.to_string();
            }
            if !ssh::is_valid_username(&username) {
                result.skipped += 1;
                result.warnings.push(format!(
                    "Skipped “{}” — “{}” is not a usable username.",
                    sanitised_name(&session.name),
                    sanitised_name(&username)
                ));
                continue;
            }
        }

        // Walk the export's folder path, creating what is missing.
        let mut parent = root.id;
        let mut key = String::new();
        for component in session.folder_path.iter().filter(|c| !c.is_empty()) {
            key.push('/');
            key.push_str(component);
            if let Some(existing) = folder_cache.get(&key) {
                parent = *existing;
                continue;
            }
            let folder = store.add_folder(HostFolder {
                id: uuid::Uuid::new_v4(),
                name: sanitised_name(component),
                parent_folder_id: Some(parent),
                sort_order: 0,
            });
            result.folders_added += 1;
            folder_cache.insert(key.clone(), folder.id);
            parent = folder.id;
        }

        let has_password = session.password.as_deref().is_some_and(|p| !p.is_empty());
        let name = sanitised_name(if session.name.is_empty() { &session.hostname } else { &session.name });
        let saved = store.add_host(Host {
            id: uuid::Uuid::new_v4(),
            name,
            hostname: session.hostname.clone(),
            port: session.port,
            username,
            // Password auth only when the export actually carried one;
            // otherwise the agent, which is what the other formats imply.
            auth_method: if has_password { AuthMethod::Password {} } else { AuthMethod::Agent {} },
            connection_type: session.connection_type,
            folder_id: Some(parent),
            jump_host_id: None,
            color_tag: None,
            tunnels: Vec::new(),
            sort_order: 0,
            notes: String::new(),
            vnc_shared_clipboard: false,
        });
        result.hosts_added += 1;

        if let (Some(password), Some(vault)) = (session.password.as_deref(), vault.as_deref_mut()) {
            if !password.is_empty() {
                vault.set(saved.secret_account_for(), password.to_string());
                result.passwords_saved += 1;
            }
        }
    }

    result
}
