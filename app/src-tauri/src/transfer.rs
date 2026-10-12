//! Backup export / restore and importing from other tools.
//!
//! The formats and the merge rules live in bhterm-core; this is the part that
//! needs a filesystem and the real vault — reading the file the user picked,
//! writing one owner-only, and turning an error into something worth reading.

use crate::paths;
use crate::vault::SharedVault;
use bhterm_core::{backup, import, SessionStore};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

/// RFC 3339, UTC, to the second — what the macOS app wrote into the envelope.
/// Hand-rolled because a whole date library for one timestamp is not worth the
/// dependency; the civil-from-days conversion is Howard Hinnant's.
fn now_rfc3339() -> String {
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0) as i64;
    rfc3339_from_unix(secs)
}

fn rfc3339_from_unix(secs: i64) -> String {
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = era * 400 + yoe + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

/// `YYYY-MM-DD-HH-MM-SS`, for a filename that sorts by when it was made.
pub fn now_stamp() -> String {
    stamp_from_unix(SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0) as i64)
}

fn stamp_from_unix(secs: i64) -> String {
    rfc3339_from_unix(secs).replace(['T', ':'], "-").replace('Z', "")
}

fn write_private(path: &Path, bytes: &[u8]) -> Result<(), String> {
    std::fs::write(path, bytes).map_err(|e| format!("could not write {}: {e}", path.display()))?;
    // A backup holding secrets must not be world-readable even for the moment
    // before the user moves it somewhere safe.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
            .map_err(|e| format!("could not restrict {}: {e}", path.display()))?;
    }
    Ok(())
}

fn load_store() -> Result<SessionStore, String> {
    SessionStore::load(&paths::sessions_file().map_err(|e| e.to_string())?).map_err(|e| e.to_string())
}

fn save_store(store: &SessionStore) -> Result<(), String> {
    store
        .save(&paths::sessions_file().map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())
}

/// Writes a backup to `path`. Including secrets forces encryption — writing
/// them in the clear would undo the point of the vault — which is why the
/// passphrase is required in that case rather than merely offered.
#[tauri::command]
pub fn backup_export(
    vault: tauri::State<'_, SharedVault>,
    path: String,
    include_secrets: bool,
    passphrase: Option<String>,
) -> Result<usize, String> {
    let store = load_store()?;
    let snapshot = include_secrets.then(|| vault.snapshot());
    let payload = backup::snapshot(&store, snapshot.as_ref());
    let count = payload.secrets.as_ref().map_or(0, |s| s.len());

    let bytes = backup::encode(
        &payload,
        include_secrets,
        passphrase.as_deref(),
        &now_rfc3339(),
        env!("CARGO_PKG_VERSION"),
    )
    .map_err(|e| e.to_string())?;
    write_private(Path::new(&path), &bytes)?;
    Ok(count)
}

/// Whether this file needs a passphrase, so the UI can ask for one only when
/// it is actually required.
#[tauri::command]
pub fn backup_is_encrypted(path: String) -> Result<bool, String> {
    let bytes = std::fs::read(&path).map_err(|e| format!("could not read {path}: {e}"))?;
    backup::is_encrypted(&bytes).map_err(|e| e.to_string())
}

/// Merges a backup in. Nothing is deleted: restoring an old archive must not
/// take away hosts added since it was made.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoreReport {
    #[serde(flatten)]
    pub summary: backup::RestoreSummary,
    /// Hosts whose private key file is not on THIS computer. A backup moved
    /// between machines — the Mac build to the Windows one, most often —
    /// carries the path it was saved with, and that path does not exist on
    /// the other side. Saying so now beats a connection that fails later for
    /// a reason nobody can see.
    pub missing_keys: Vec<String>,
}

#[tauri::command]
pub fn backup_restore(
    vault: tauri::State<'_, SharedVault>,
    path: String,
    passphrase: Option<String>,
) -> Result<RestoreReport, String> {
    let bytes = std::fs::read(&path).map_err(|e| format!("could not read {path}: {e}"))?;
    let payload = backup::decode(&bytes, passphrase.as_deref()).map_err(|e| e.to_string())?;

    let mut store = load_store()?;
    // One vault write for the whole archive, and the store is only saved once
    // the secrets are in — a half-restore that lost the passwords would be
    // worse than one that failed outright.
    let summary = vault.mutate(|secrets| backup::restore(&payload, &mut store, Some(secrets)))?;
    save_store(&store)?;

    let missing_keys = store
        .hosts
        .iter()
        .filter_map(|h| match &h.auth_method {
            bhterm_core::AuthMethod::PrivateKey { path } if !key_exists(path) => {
                Some(format!("{} ({})", h.name, path))
            }
            _ => None,
        })
        .collect();
    Ok(RestoreReport { summary, missing_keys })
}

/// `~` is the user's own home on whichever platform this is, so it is expanded
/// before the file is looked for rather than treated as a literal directory.
fn key_exists(path: &str) -> bool {
    if path.is_empty() {
        return false;
    }
    let expanded = match path.strip_prefix("~/").or_else(|| path.strip_prefix("~\\")) {
        Some(rest) => match dirs::home_dir() {
            Some(home) => home.join(rest),
            None => return false,
        },
        None => std::path::PathBuf::from(path),
    };
    expanded.exists()
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportPreview {
    pub source: String,
    pub hosts: usize,
    pub warnings: Vec<String>,
}

/// Reads the file and says what it is, without changing anything — an import
/// of 700 hosts deserves a confirmation step.
#[tauri::command]
pub fn import_preview(path: String) -> Result<ImportPreview, String> {
    let (parsed, _) = read_import(&path)?;
    Ok(ImportPreview {
        source: parsed.source.label().to_string(),
        hosts: parsed.sessions.len(),
        warnings: parsed.warnings,
    })
}

#[tauri::command]
pub fn import_apply(
    vault: tauri::State<'_, SharedVault>,
    path: String,
) -> Result<import::ImportResult, String> {
    let (parsed, _) = read_import(&path)?;
    let mut store = load_store()?;
    let result = vault.mutate(|secrets| {
        import::commit(
            parsed.source,
            &parsed.sessions,
            parsed.warnings,
            &mut store,
            Some(secrets),
            &whoami(),
        )
    })?;
    save_store(&store)?;
    Ok(result)
}

fn read_import(path: &str) -> Result<(import::Parsed, String), String> {
    let bytes = std::fs::read(path).map_err(|e| format!("could not read {path}: {e}"))?;
    // These files come off Windows: UTF-16 with a BOM is common, and lossy
    // decoding is right for a session list — one odd byte in a name must not
    // cost the user the other 734 hosts.
    let text = decode_text(&bytes);
    let name = Path::new(path).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let parsed = import::parse(&text, &name).map_err(|e| e.to_string())?;
    Ok((parsed, name))
}

fn decode_text(bytes: &[u8]) -> String {
    match bytes {
        [0xFF, 0xFE, rest @ ..] => decode_utf16(rest, u16::from_le_bytes),
        [0xFE, 0xFF, rest @ ..] => decode_utf16(rest, u16::from_be_bytes),
        [0xEF, 0xBB, 0xBF, rest @ ..] => String::from_utf8_lossy(rest).into_owned(),
        _ => String::from_utf8_lossy(bytes).into_owned(),
    }
}

fn decode_utf16(bytes: &[u8], to_u16: fn([u8; 2]) -> u16) -> String {
    let units: Vec<u16> = bytes.chunks_exact(2).map(|c| to_u16([c[0], c[1]])).collect();
    String::from_utf16_lossy(&units)
}

/// The account name to assume for an export that records no username.
fn whoami() -> String {
    std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_else(|_| "root".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamps_match_known_epochs() {
        assert_eq!(rfc3339_from_unix(0), "1970-01-01T00:00:00Z");
        assert_eq!(rfc3339_from_unix(1_760_000_000), "2025-10-09T08:53:20Z");
        // A leap day, which is where a hand-rolled calendar goes wrong.
        assert_eq!(rfc3339_from_unix(1_709_164_800), "2024-02-29T00:00:00Z");
        assert_eq!(rfc3339_from_unix(951_782_400), "2000-02-29T00:00:00Z");
        // 2100 is NOT a leap year, so Feb 28 is followed by Mar 1.
        assert_eq!(rfc3339_from_unix(4_107_456_000), "2100-02-28T00:00:00Z");
        assert_eq!(rfc3339_from_unix(4_107_542_400), "2100-03-01T00:00:00Z");
    }

    #[test]
    fn a_log_filename_sorts_by_time_and_has_no_colons() {
        // A colon is a path separator on some systems and a nuisance on the
        // rest, so it must not reach a filename.
        let stamp = stamp_from_unix(1_760_000_000);
        assert_eq!(stamp, "2025-10-09-08-53-20");
        assert!(!stamp.contains(':') && !stamp.contains('T') && !stamp.contains('Z'));
    }

    #[test]
    fn utf16_and_bom_prefixed_files_decode() {
        let mut le = vec![0xFF, 0xFE];
        for u in "Host x".encode_utf16() {
            le.extend_from_slice(&u.to_le_bytes());
        }
        assert_eq!(decode_text(&le), "Host x");

        let mut be = vec![0xFE, 0xFF];
        for u in "Host x".encode_utf16() {
            be.extend_from_slice(&u.to_be_bytes());
        }
        assert_eq!(decode_text(&be), "Host x");

        assert_eq!(decode_text(b"\xEF\xBB\xBFHost x"), "Host x");
        assert_eq!(decode_text(b"Host x"), "Host x");
    }
}
