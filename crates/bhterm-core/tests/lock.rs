//! The master-password lock. A bug here either locks the user out of every
//! saved credential or lets the wrong password in.

use bhterm_core::lock::{self, LockError};
use bhterm_core::SecretVault;

const KEY: &str = "3f9a0c1e5b7d2468ace013579bdf2468ace013579bdf2468ace013579bdf2468";

#[test]
fn the_right_password_gives_back_the_same_key() {
    let bytes = lock::wrap(KEY, "correct horse").unwrap();
    assert_eq!(lock::unwrap(&bytes, "correct horse").unwrap(), KEY);
}

#[test]
fn a_wrong_password_is_refused_not_turned_into_a_wrong_key() {
    let bytes = lock::wrap(KEY, "correct horse").unwrap();
    assert_eq!(lock::unwrap(&bytes, "correct hors"), Err(LockError::WrongPassword));
    assert_eq!(lock::unwrap(&bytes, "Correct horse"), Err(LockError::WrongPassword));
    assert_eq!(lock::unwrap(&bytes, ""), Err(LockError::WrongPassword));
}

#[test]
fn the_lock_file_never_contains_the_key_or_the_password() {
    let bytes = lock::wrap(KEY, "correct horse").unwrap();
    let text = String::from_utf8(bytes).unwrap();
    assert!(!text.contains(KEY));
    assert!(!text.contains("correct horse"));
}

#[test]
fn each_wrap_is_salted_afresh() {
    // Same key, same password, different files — so two installs that chose
    // the same password cannot be told apart, or attacked together.
    let a = lock::wrap(KEY, "correct horse").unwrap();
    let b = lock::wrap(KEY, "correct horse").unwrap();
    assert_ne!(a, b);
}

#[test]
fn short_passwords_are_refused_counting_characters_not_bytes() {
    assert_eq!(lock::wrap(KEY, "1234567"), Err(LockError::TooShort));
    assert!(lock::wrap(KEY, "12345678").is_ok());
    // Eight Bengali letters are 24 bytes but still eight characters: allowed.
    let eight = "ক".repeat(8);
    assert_eq!(eight.len(), 24);
    assert!(lock::wrap(KEY, &eight).is_ok());
    // Seven are not, even though 21 bytes would pass a byte count.
    assert_eq!(lock::check_master(&"ক".repeat(7)), Err(LockError::TooShort));
}

#[test]
fn a_tampered_lock_is_refused() {
    let bytes = lock::wrap(KEY, "correct horse").unwrap();
    let mut v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();

    // A flipped byte in the sealed key fails authentication.
    let sealed = v["sealed"].as_str().unwrap().to_string();
    let mut raw = base64_decode(&sealed);
    let last = raw.len() - 1;
    raw[last] ^= 1;
    v["sealed"] = base64_encode(&raw).into();
    let tampered = serde_json::to_vec(&v).unwrap();
    assert_eq!(lock::unwrap(&tampered, "correct horse"), Err(LockError::WrongPassword));
}

#[test]
fn a_lock_asking_for_absurd_work_is_refused_before_any_is_done() {
    let bytes = lock::wrap(KEY, "correct horse").unwrap();
    for iterations in [0u64, 4_000_000_000] {
        let mut v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        v["kdfIterations"] = iterations.into();
        let doctored = serde_json::to_vec(&v).unwrap();
        // Returns at once rather than hanging for hours.
        assert_eq!(lock::unwrap(&doctored, "correct horse"), Err(LockError::Malformed));
    }
}

#[test]
fn something_that_is_not_a_lock_file_is_malformed() {
    assert_eq!(lock::unwrap(b"not json", "correct horse"), Err(LockError::Malformed));
    assert_eq!(lock::unwrap(br#"{"format":"other"}"#, "correct horse"), Err(LockError::Malformed));
    let mut v: serde_json::Value =
        serde_json::from_slice(&lock::wrap(KEY, "correct horse").unwrap()).unwrap();
    v["version"] = 99.into();
    assert_eq!(
        lock::unwrap(&serde_json::to_vec(&v).unwrap(), "correct horse"),
        Err(LockError::Malformed)
    );
}

#[test]
fn the_unwrapped_key_opens_the_vault_it_was_sealing() {
    // End to end: a vault saved under the plain key opens under the key
    // recovered through the lock — the property everything else rests on.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("secrets.json");
    let mut vault = SecretVault::new();
    vault.set("host-1", "s3cret");
    vault.save(&path, KEY).unwrap();

    let bytes = lock::wrap(KEY, "correct horse").unwrap();
    let recovered = lock::unwrap(&bytes, "correct horse").unwrap();
    assert_eq!(SecretVault::load(&path, &recovered).unwrap().get("host-1"), Some("s3cret"));
}

fn base64_decode(s: &str) -> Vec<u8> {
    bhterm_core::crypto::b64_decode(s).unwrap()
}

fn base64_encode(b: &[u8]) -> String {
    bhterm_core::crypto::b64_encode(b)
}
