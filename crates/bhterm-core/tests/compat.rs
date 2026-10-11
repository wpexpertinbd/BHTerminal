//! The replacement must read what the shipped Swift app wrote. If this file
//! ever fails, someone's whole host list is about to vanish on upgrade.

use bhterm_core::{AuthMethod, ConnectionType, SessionStore, TreeItem};
use std::path::Path;

fn fixture() -> SessionStore {
    SessionStore::load(Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures_sessions.json"
    )))
    .expect("the Swift-written fixture must load")
}

#[test]
fn reads_swift_enum_encoding() {
    let store = fixture();
    assert_eq!(store.hosts.len(), 3);

    // Swift encodes an enum case as an object keyed by the case name, so a
    // payload-free case is `{"password":{}}` — NOT the bare string serde
    // would emit for a unit variant.
    let by_name = |n: &str| store.hosts.iter().find(|h| h.name == n).unwrap().clone();
    assert!(matches!(by_name("desk").auth_method, AuthMethod::Password {}));
    assert!(matches!(
        by_name("legacy-no-connectionType").auth_method,
        AuthMethod::Agent {}
    ));
    match by_name("s1").auth_method {
        AuthMethod::PrivateKey { ref path } => assert_eq!(path, "~/.ssh/id_ed25519"),
        ref other => panic!("expected a private key, got {other:?}"),
    }
}

#[test]
fn honours_swifts_capital_id_keys() {
    // Swift writes `folderID`, which camelCase would render `folderId`; the
    // mismatch silently orphans every host from its folder.
    let store = fixture();
    let s1 = store.hosts.iter().find(|h| h.name == "s1").unwrap();
    assert!(s1.folder_id.is_some(), "folderID must map to folder_id");

    let roots = store.tree();
    assert_eq!(roots.len(), 3, "1 folder + 2 hosts that sit outside it");
    let folder = roots
        .iter()
        .find(|i| matches!(i, TreeItem::Folder { .. }))
        .expect("the folder must be a root");
    match folder {
        TreeItem::Folder { children, folder } => {
            assert_eq!(folder.name, "BiswasHost");
            assert_eq!(children.len(), 1, "s1 must be nested inside its folder");
        }
        _ => unreachable!(),
    }
}

#[test]
fn tolerates_fields_added_after_hosts_were_already_saved() {
    // `connectionType` postdates the first released sessions.json. One
    // missing key must never drop the whole document.
    let store = fixture();
    let legacy = store
        .hosts
        .iter()
        .find(|h| h.name == "legacy-no-connectionType")
        .unwrap();
    assert_eq!(legacy.connection_type, ConnectionType::Ssh);
    assert_eq!(legacy.port, 22);
    assert!(legacy.tunnels.is_empty());
}

#[test]
fn round_trips_without_losing_anything() {
    let store = fixture();
    let json = serde_json::to_vec(&store).unwrap();
    let again: SessionStore = serde_json::from_slice(&json).unwrap();
    assert_eq!(again, store);

    // And the keys we re-emit are the ones Swift reads back.
    let text = String::from_utf8(json).unwrap();
    for key in ["folderID", "authMethod", "connectionType", "vncSharedClipboard"] {
        assert!(text.contains(key), "missing key {key} in re-serialised output");
    }
}

#[test]
fn a_missing_file_is_a_first_run_not_a_failure() {
    let store = SessionStore::load(Path::new("/nonexistent/bhterminal/sessions.json")).unwrap();
    assert!(store.hosts.is_empty());
}

#[test]
fn a_corrupt_file_is_an_error_not_an_empty_store() {
    // Silently starting empty would invite overwriting a real host list on
    // the next save.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sessions.json");
    std::fs::write(&path, b"{ not json").unwrap();
    assert!(SessionStore::load(&path).is_err());
}
