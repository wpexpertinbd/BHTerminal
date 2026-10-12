//! Deciding whether a GitHub release is an update, and which file to offer.
//!
//! The webview does the fetch — it already has an HTTP stack, and the GitHub
//! API allows cross-origin reads — but every decision made from the response
//! lives here, where it can be tested and where a tampered response cannot
//! reach past it: the version comparison, which asset belongs to this machine,
//! and above all the allow-list that keeps the download on GitHub's own hosts.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize)]
pub struct Asset {
    pub name: String,
    #[serde(rename = "browser_download_url")]
    pub url: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Release {
    pub tag_name: String,
    #[serde(default)]
    pub assets: Vec<Asset>,
    #[serde(default)]
    pub html_url: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Available {
    pub version: String,
    /// The installer for this machine, when the release has one.
    pub download_url: Option<String>,
    pub download_name: Option<String>,
    /// Always set: somewhere to send the user if there is no matching asset.
    pub page_url: String,
}

/// Numeric, component-wise, so 1.10.0 is newer than 1.9.3 — which a string
/// comparison gets backwards.
pub fn is_newer(candidate: &str, current: &str) -> bool {
    let parts = |s: &str| -> Vec<u64> {
        s.trim_start_matches(['v', 'V', ' '])
            .split(['.', '-', '+'])
            .map(|p| p.parse().unwrap_or(0))
            .collect()
    };
    let (a, b) = (parts(candidate), parts(current));
    for i in 0..a.len().max(b.len()) {
        let (x, y) = (a.get(i).copied().unwrap_or(0), b.get(i).copied().unwrap_or(0));
        if x != y {
            return x > y;
        }
    }
    false
}

/// The download must be HTTPS on one of GitHub's own hosts. A tampered or
/// man-in-the-middled API response must not be able to point the updater at
/// an arbitrary binary, and the lookalike cases are the ones to get right:
/// `github.com.evil.example` does not end with `.github.com`, and anything
/// carrying userinfo (`https://github.com@evil.example/`) is refused outright
/// rather than parsed.
pub fn is_trusted_download(raw: &str) -> bool {
    let Some(rest) = raw.strip_prefix("https://") else { return false };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    if authority.is_empty() || authority.contains('@') {
        return false;
    }
    // Strip a port, and reject an IP-literal host outright.
    let host = match authority.rsplit_once(':') {
        Some((h, port)) if !port.is_empty() && port.chars().all(|c| c.is_ascii_digit()) => h,
        Some(_) => return false,
        None => authority,
    }
    .to_ascii_lowercase();

    host == "github.com"
        || host == "objects.githubusercontent.com"
        || host.ends_with(".github.com")
        || host.ends_with(".githubusercontent.com")
}

/// Which installer belongs to this machine. The extension order is the
/// preference for that platform: a .pkg can quit and relaunch the app where a
/// .dmg cannot, and a .deb is right far more often than an .rpm.
fn wanted_extensions(os: &str) -> &'static [&'static str] {
    match os {
        "macos" => &[".pkg", ".dmg"],
        "windows" => &["-setup.exe", ".msi", ".exe"],
        _ => &[".deb", ".rpm", ".AppImage"],
    }
}

/// Release assets name the same architecture several ways — `aarch64` in an
/// rpm, `arm64` in a deb, `x64` in a dmg, `amd64` in a deb.
fn arch_aliases(arch: &str) -> &'static [&'static str] {
    match arch {
        "aarch64" | "arm64" => &["aarch64", "arm64"],
        _ => &["x86_64", "amd64", "x64"],
    }
}

pub fn pick_asset<'a>(assets: &'a [Asset], os: &str, arch: &str) -> Option<&'a Asset> {
    let aliases = arch_aliases(arch);
    for ext in wanted_extensions(os) {
        let matching: Vec<&Asset> = assets
            .iter()
            .filter(|a| a.name.ends_with(ext) && is_trusted_download(&a.url))
            .collect();
        // Prefer this machine's architecture; fall back to a single
        // unambiguous asset, but never guess between two of the wrong one.
        if let Some(hit) = matching
            .iter()
            .find(|a| aliases.iter().any(|al| a.name.to_ascii_lowercase().contains(al)))
        {
            return Some(hit);
        }
        if matching.len() == 1
            && !["aarch64", "arm64", "x86_64", "amd64", "x64"]
                .iter()
                .any(|a| matching[0].name.to_ascii_lowercase().contains(a))
        {
            return Some(matching[0]);
        }
    }
    None
}

/// `None` means "not an update" — the only honest answers are "newer version,
/// here it is" and "nothing newer". A failed check is the caller's to report,
/// and must never be reported as up to date.
pub fn evaluate(release: &Release, current: &str, os: &str, arch: &str) -> Option<Available> {
    let version = release.tag_name.trim_start_matches(['v', 'V', ' ']).to_string();
    if !is_newer(&version, current) {
        return None;
    }
    let asset = pick_asset(&release.assets, os, arch);
    let page_url = if is_trusted_download(&release.html_url) {
        release.html_url.clone()
    } else {
        // A tampered html_url is replaced, not followed.
        "https://github.com/wpexpertinbd/BHTerminal/releases/latest".into()
    };
    Some(Available {
        version,
        download_url: asset.map(|a| a.url.clone()),
        download_name: asset.map(|a| a.name.clone()),
        page_url,
    })
}
