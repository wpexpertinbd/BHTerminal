//! Deleting folders: the cascade must take everything underneath, and must
//! finish — even on a folder tree that loops (imported or hand-edited data).

use bhterm_core::model::{Host, HostFolder};
use bhterm_core::SessionStore;
use std::sync::mpsc;
use std::time::Duration;
use uuid::Uuid;

fn folder(name: &str, parent: Option<Uuid>) -> HostFolder {
    HostFolder { id: Uuid::new_v4(), name: name.into(), parent_folder_id: parent, sort_order: 0 }
}

fn host_in(store: &mut SessionStore, name: &str, folder: Option<Uuid>) -> Host {
    let mut h = Host::new(name, "10.0.0.1", "root");
    h.folder_id = folder;
    store.add_host(h)
}

/// Runs `delete_folder` on a thread with a deadline, so a regression to the
/// endless walk fails this test instead of hanging the whole suite.
fn delete_within_deadline(mut store: SessionStore, id: Uuid) -> (SessionStore, Vec<String>) {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let freed = store.delete_folder(id);
        let _ = tx.send((store, freed));
    });
    rx.recv_timeout(Duration::from_secs(5)).expect("delete_folder did not finish — it loops")
}

#[test]
fn deleting_a_folder_takes_every_sub_folder_and_host_underneath() {
    let mut store = SessionStore::default();
    let top = folder("Production", None);
    let mid = folder("Web", Some(top.id));
    let deep = folder("Deep", Some(mid.id));
    let other = folder("Other", None);
    for f in [&top, &mid, &deep, &other] {
        store.folders.push(f.clone());
    }
    let a = host_in(&mut store, "a", Some(top.id));
    let b = host_in(&mut store, "b", Some(deep.id));
    let keep = host_in(&mut store, "keep", Some(other.id));

    let (store, freed) = delete_within_deadline(store, top.id);
    assert_eq!(store.folders.len(), 1, "only the unrelated folder is left");
    assert_eq!(store.folders[0].id, other.id);
    assert_eq!(store.hosts.len(), 1);
    assert_eq!(store.hosts[0].id, keep.id);
    for gone in [&a, &b] {
        assert!(freed.contains(&gone.secret_account_for()), "{}'s password must be freed", gone.name);
        assert!(freed.contains(&gone.passphrase_account_for()));
    }
    assert!(!freed.contains(&keep.secret_account_for()));
}

#[test]
fn a_folder_loop_is_deleted_without_hanging() {
    let mut store = SessionStore::default();
    let mut a = folder("Loop A", None);
    let b = folder("Loop B", Some(a.id));
    a.parent_folder_id = Some(b.id); // A → B → A
    store.folders.push(a.clone());
    store.folders.push(b.clone());
    let h = host_in(&mut store, "in-the-loop", Some(b.id));

    let (store, freed) = delete_within_deadline(store, a.id);
    assert!(store.folders.is_empty(), "both folders of the loop go");
    assert!(store.hosts.is_empty());
    assert!(freed.contains(&h.secret_account_for()));
    // Each account is freed once, not once per trip round the loop.
    assert_eq!(freed.len(), 2);
}

#[test]
fn a_folder_that_is_its_own_parent_is_deleted_without_hanging() {
    let mut store = SessionStore::default();
    let mut selfish = folder("Self", None);
    selfish.parent_folder_id = Some(selfish.id);
    store.folders.push(selfish.clone());
    host_in(&mut store, "inside", Some(selfish.id));

    let (store, _) = delete_within_deadline(store, selfish.id);
    assert!(store.folders.is_empty());
    assert!(store.hosts.is_empty());
}
