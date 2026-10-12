//! The update rules, including the lookalike hosts the allow-list exists for.

use bhterm_core::update::{self, Asset, Release};

fn asset(name: &str) -> Asset {
    serde_json::from_value(serde_json::json!({
        "name": name,
        "browser_download_url": format!("https://github.com/wpexpertinbd/BHTerminal/releases/download/v2.1.0/{name}"),
    }))
    .unwrap()
}

fn release(tag: &str, names: &[&str]) -> Release {
    Release {
        tag_name: tag.into(),
        assets: names.iter().map(|n| asset(n)).collect(),
        html_url: format!("https://github.com/wpexpertinbd/BHTerminal/releases/tag/{tag}"),
    }
}

#[test]
fn version_compare_is_numeric() {
    assert!(update::is_newer("1.10.0", "1.9.3"), "a string compare gets this backwards");
    assert!(update::is_newer("2.0.0", "1.3.2"));
    assert!(update::is_newer("v2.0.1", "2.0.0"), "a leading v is tolerated");
    assert!(!update::is_newer("2.0.0", "2.0.0"));
    assert!(!update::is_newer("1.3.2", "2.0.0"));
    // Shorter is not newer: 2.0 == 2.0.0.
    assert!(!update::is_newer("2.0", "2.0.0"));
    assert!(update::is_newer("2.0.1", "2.0"));
}

#[test]
fn only_githubs_own_hosts_over_https() {
    assert!(update::is_trusted_download("https://github.com/x/y/releases/download/v1/a.pkg"));
    assert!(update::is_trusted_download("https://objects.githubusercontent.com/a"));
    assert!(update::is_trusted_download("https://release-assets.githubusercontent.com/a"));

    assert!(!update::is_trusted_download("http://github.com/a"), "plain http");
    assert!(!update::is_trusted_download("https://github.com.evil.example/a"), "suffix lookalike");
    assert!(!update::is_trusted_download("https://notgithub.com/a"));
    assert!(!update::is_trusted_download("https://evil.example/github.com/a"));
    assert!(!update::is_trusted_download("https://github.com@evil.example/a"), "userinfo");
    assert!(!update::is_trusted_download("https://93.184.216.34/a"), "bare IP");
    assert!(!update::is_trusted_download("file:///tmp/a.pkg"));
    assert!(!update::is_trusted_download("https:///a"), "empty host");
    // A port is allowed as long as the host still matches.
    assert!(update::is_trusted_download("https://github.com:443/a"));
}

#[test]
fn picks_this_machines_installer() {
    let names = [
        "BHTerminal-2.1.0-1.aarch64.rpm",
        "BHTerminal-2.1.0-1.x86_64.rpm",
        "BHTerminal_2.1.0_aarch64.dmg",
        "BHTerminal_2.1.0_amd64.deb",
        "BHTerminal_2.1.0_arm64-setup.exe",
        "BHTerminal_2.1.0_arm64.deb",
        "BHTerminal_2.1.0_x64-setup.exe",
        "BHTerminal_2.1.0_x64.dmg",
    ];
    let assets: Vec<Asset> = names.iter().map(|n| asset(n)).collect();
    let pick = |os, arch| update::pick_asset(&assets, os, arch).map(|a| a.name.clone()).unwrap();

    assert_eq!(pick("macos", "aarch64"), "BHTerminal_2.1.0_aarch64.dmg");
    assert_eq!(pick("macos", "x86_64"), "BHTerminal_2.1.0_x64.dmg");
    assert_eq!(pick("windows", "aarch64"), "BHTerminal_2.1.0_arm64-setup.exe");
    assert_eq!(pick("windows", "x86_64"), "BHTerminal_2.1.0_x64-setup.exe");
    // .deb is preferred over .rpm, and amd64 is this machine's x86_64.
    assert_eq!(pick("linux", "x86_64"), "BHTerminal_2.1.0_amd64.deb");
    assert_eq!(pick("linux", "aarch64"), "BHTerminal_2.1.0_arm64.deb");
}

#[test]
fn a_pkg_wins_over_a_dmg_on_macos() {
    let assets = [asset("BHTerminal-2.1.0.pkg"), asset("BHTerminal_2.1.0_aarch64.dmg")];
    assert_eq!(
        update::pick_asset(&assets, "macos", "aarch64").unwrap().name,
        "BHTerminal-2.1.0.pkg",
        "the .pkg can quit and relaunch the app; the .dmg cannot"
    );
}

#[test]
fn never_guesses_between_two_wrong_architectures() {
    let assets = [asset("BHTerminal_2.1.0_x64.dmg"), asset("BHTerminal_2.1.0_amd64.deb")];
    assert!(
        update::pick_asset(&assets, "macos", "aarch64").is_none(),
        "an Intel dmg must not be offered to an Apple-silicon Mac"
    );
}

#[test]
fn an_untrusted_asset_url_is_not_offered() {
    let evil = Asset {
        name: "BHTerminal_2.1.0_aarch64.dmg".into(),
        url: "https://evil.example/BHTerminal_2.1.0_aarch64.dmg".into(),
    };
    assert!(update::pick_asset(&[evil], "macos", "aarch64").is_none());
}

#[test]
fn evaluate_reports_only_a_genuinely_newer_release() {
    let r = release("v2.1.0", &["BHTerminal_2.1.0_aarch64.dmg"]);
    let found = update::evaluate(&r, "2.0.0", "macos", "aarch64").unwrap();
    assert_eq!(found.version, "2.1.0");
    assert_eq!(found.download_name.as_deref(), Some("BHTerminal_2.1.0_aarch64.dmg"));

    assert!(update::evaluate(&r, "2.1.0", "macos", "aarch64").is_none());
    assert!(update::evaluate(&r, "2.2.0", "macos", "aarch64").is_none());
}

#[test]
fn a_release_with_no_matching_asset_still_offers_the_page() {
    let r = release("v2.1.0", &["BHTerminal_2.1.0_x64-setup.exe"]);
    let found = update::evaluate(&r, "2.0.0", "macos", "aarch64").unwrap();
    assert_eq!(found.download_url, None);
    assert!(found.page_url.starts_with("https://github.com/"));
}

#[test]
fn a_tampered_page_url_is_replaced_not_followed() {
    let mut r = release("v2.1.0", &[]);
    r.html_url = "https://evil.example/pwn".into();
    let found = update::evaluate(&r, "2.0.0", "linux", "x86_64").unwrap();
    assert_eq!(found.page_url, "https://github.com/wpexpertinbd/BHTerminal/releases/latest");
}
