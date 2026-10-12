//! The credential vault and the backup archive — the two places where a bug
//! loses or leaks the user's passwords.

use bhterm_core::backup::{self, Payload};
use bhterm_core::model::{Host, HostFolder, Snippet};
use bhterm_core::{SecretVault, SessionStore};
use uuid::Uuid;

fn sample() -> (SessionStore, SecretVault) {
    let folder = HostFolder {
        id: Uuid::new_v4(),
        name: "BiswasHost".into(),
        parent_folder_id: None,
        sort_order: 0,
    };
    let mut host = Host::new("s1", "s1.example.com", "root");
    host.folder_id = Some(folder.id);

    let mut store = SessionStore::default();
    store.folders.push(folder);
    let host = store.add_host(host);
    store.add_snippet(Snippet {
        id: Uuid::new_v4(),
        name: "disk".into(),
        command: "df -h".into(),
        sort_order: 0,
    });

    let mut vault = SecretVault::new();
    vault.set(host.secret_account_for(), "s3cret-password");
    vault.set(host.passphrase_account_for(), "s3cret-passphrase");
    (store, vault)
}

// ───────────────────────── vault ─────────────────────────

#[test]
fn vault_round_trips_through_an_encrypted_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("secrets.json");
    let (_, vault) = sample();
    vault.save(&path, "a-good-passphrase").unwrap();

    let loaded = SecretVault::load(&path, "a-good-passphrase").unwrap();
    assert_eq!(loaded, vault);
    assert_eq!(loaded.len(), 2);
}

#[test]
fn the_vault_file_never_contains_a_secret_in_cleartext() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("secrets.json");
    let (_, vault) = sample();
    vault.save(&path, "a-good-passphrase").unwrap();

    let raw = std::fs::read_to_string(&path).unwrap();
    assert!(!raw.contains("s3cret-password"));
    assert!(!raw.contains("s3cret-passphrase"));
    assert!(!raw.contains("host."), "account names leak which hosts exist");
}

#[test]
fn vault_is_owner_only_on_disk() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("secrets.json");
    let (_, vault) = sample();
    vault.save(&path, "pw").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "other accounts must not be able to read it");
    }
}

#[test]
fn a_wrong_vault_passphrase_fails_rather_than_returning_an_empty_vault() {
    // "Unreadable" must never look like "empty" — that is how a save would
    // overwrite every stored credential with nothing.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("secrets.json");
    let (_, vault) = sample();
    vault.save(&path, "the-real-one").unwrap();
    assert!(SecretVault::load(&path, "not-it").is_err());
}

#[test]
fn an_absent_vault_is_a_first_run() {
    let dir = tempfile::tempdir().unwrap();
    let v = SecretVault::load(&dir.path().join("nope.json"), "pw").unwrap();
    assert!(v.is_empty());
}

// ───────────────────────── backup ─────────────────────────

#[test]
fn plain_backup_round_trips_and_carries_no_secrets() {
    let (store, vault) = sample();
    let payload = backup::snapshot(&store, Some(&vault));
    let bytes = backup::encode(&payload, false, None, "2026-10-12T00:00:00Z", "2.0.0").unwrap();

    assert!(!backup::is_encrypted(&bytes).unwrap());
    let decoded = backup::decode(&bytes, None).unwrap();
    assert_eq!(decoded.hosts.len(), 1);
    assert_eq!(decoded.folders.len(), 1);
    assert_eq!(decoded.snippets.len(), 1);
    assert!(decoded.secrets.is_none(), "a plain backup must not carry secrets");
    assert!(!String::from_utf8_lossy(&bytes).contains("s3cret"));
}

#[test]
fn encrypted_backup_round_trips_and_hides_everything() {
    let (store, vault) = sample();
    let payload = backup::snapshot(&store, Some(&vault));
    let bytes = backup::encode(&payload, true, Some("correct horse"), "t", "2.0.0").unwrap();

    assert!(backup::is_encrypted(&bytes).unwrap());
    let text = String::from_utf8_lossy(&bytes);
    assert!(!text.contains("s3cret-password"));
    assert!(!text.contains("s1.example.com"), "hostnames leak the estate too");

    let decoded = backup::decode(&bytes, Some("correct horse")).unwrap();
    assert_eq!(decoded.secrets.as_ref().unwrap().len(), 2);
    assert_eq!(decoded.hosts[0].hostname, "s1.example.com");
}

#[test]
fn wrong_passphrase_and_tampering_both_fail_closed() {
    let (store, vault) = sample();
    let payload = backup::snapshot(&store, Some(&vault));
    let bytes = backup::encode(&payload, true, Some("the-real-one"), "t", "2.0.0").unwrap();

    assert!(backup::decode(&bytes, Some("not-it")).is_err());
    assert_eq!(
        backup::decode(&bytes, None).unwrap_err(),
        backup::BackupError::NeedsPassphrase
    );

    // AES-GCM authenticates, so a flipped byte must be refused rather than
    // restoring attacker-chosen hosts.
    let mut text = String::from_utf8(bytes).unwrap();
    let i = text.find("\"sealed\"").unwrap() + 20;
    let ch = text.as_bytes()[i] as char;
    text.replace_range(i..i + 1, if ch == 'A' { "B" } else { "A" });
    assert!(backup::decode(text.as_bytes(), Some("the-real-one")).is_err());
}

#[test]
fn restore_merges_and_never_deletes() {
    let (source, vault) = sample();
    let payload = backup::snapshot(&source, Some(&vault));

    // A store that has moved on since the backup: one host renamed, one new.
    let mut target = source.clone();
    target.hosts[0].name = "renamed-since-backup".into();
    let newer = target.add_host(Host::new("added-later", "new.example.com", "root"));

    let mut into_vault = SecretVault::new();
    let summary = backup::restore(&payload, &mut target, Some(&mut into_vault));

    assert_eq!(summary.hosts_updated, 1);
    assert_eq!(summary.hosts_added, 0);
    assert_eq!(summary.secrets_restored, 2);
    assert_eq!(target.hosts[0].name, "s1", "the backup's copy wins on a merge");
    assert!(
        target.host(newer.id).is_some(),
        "restoring an old backup must not delete hosts added since"
    );
}

#[test]
fn a_file_that_is_not_a_backup_is_rejected() {
    assert_eq!(
        backup::is_encrypted(br#"{"hello":"world"}"#).unwrap_err(),
        backup::BackupError::NotABackup
    );
}

#[test]
fn snapshot_skips_secrets_belonging_to_deleted_hosts() {
    let (store, mut vault) = sample();
    vault.set("host.DEAD-HOST-NO-LONGER-SAVED", "stale");
    let payload = backup::snapshot(&store, Some(&vault));
    let secrets = payload.secrets.unwrap();
    assert_eq!(secrets.len(), 2);
    assert!(!secrets.contains_key("host.DEAD-HOST-NO-LONGER-SAVED"));
}

#[test]
fn an_empty_payload_is_harmless() {
    let p = Payload::default();
    let bytes = backup::encode(&p, false, None, "t", "2.0.0").unwrap();
    assert_eq!(backup::decode(&bytes, None).unwrap().hosts.len(), 0);
}

#[test]
fn a_secret_follows_its_destination_not_its_label() {
    let base = Host::new("s1", "s1.example.com", "root");
    let mut same = base.clone();
    same.name = "renamed".into();
    same.notes = "moved folders, added notes".into();
    same.folder_id = Some(Uuid::new_v4());
    assert!(base.same_destination(&same), "a rename must keep the saved password");

    let changes: [fn(&mut Host); 5] = [
        |h| h.hostname = "attacker.example".into(),
        |h| h.port = 2222,
        |h| h.username = "admin".into(),
        |h| h.connection_type = bhterm_core::ConnectionType::Vnc,
        |h| h.jump_host_id = Some(Uuid::new_v4()),
    ];
    for change in changes {
        let mut moved = base.clone();
        change(&mut moved);
        assert!(!base.same_destination(&moved));
    }
}
