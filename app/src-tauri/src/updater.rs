//! The two halves of the update check that must not live in the webview.
//!
//! The fetch itself happens in the frontend (the GitHub API is CORS-readable
//! and the webview already has an HTTP stack), but the response is parsed and
//! judged here, and the download is opened only after the URL has been checked
//! again — so nothing a tampered response says can pick where the user is sent.
//!
//! ⚠️ This is why `connect-src` in tauri.conf.json names `https://api.github.com`.
//! That one origin is the whole network reach of the webview; everything else
//! is still refused by the CSP, and no remote content is ever loaded into it.
//! The alternative — an HTTP client in Rust — would mean carrying a TLS stack
//! for one request a day, and would not make the webview any more contained.

use bhterm_core::update::{self, Available, Release};
use tauri_plugin_opener::OpenerExt;

/// The repository the check reads. Not configurable at runtime: a settable
/// update source is a way to redirect someone's installer.
pub const RELEASES_API: &str = "https://api.github.com/repos/wpexpertinbd/BHTerminal/releases/latest";
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateQuery {
    pub url: String,
    pub current_version: String,
    /// Where to send someone who just wants to look.
    pub releases_page: String,
}

/// Where to look and what version we are, so the frontend holds neither.
#[tauri::command]
pub fn update_query() -> UpdateQuery {
    UpdateQuery {
        url: RELEASES_API.into(),
        current_version: env!("CARGO_PKG_VERSION").into(),
        releases_page: "https://github.com/wpexpertinbd/BHTerminal/releases/latest".into(),
    }
}

/// `None` means there is nothing newer. A check that FAILED is an error, which
/// the caller must not report as up to date.
#[tauri::command]
pub fn update_evaluate(body: String) -> Result<Option<Available>, String> {
    let release: Release =
        serde_json::from_str(&body).map_err(|_| "GitHub's reply was not a release".to_string())?;
    Ok(update::evaluate(
        &release,
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH,
    ))
}

/// Hands the URL to the system browser rather than downloading and running an
/// installer. The app is self-signed, so a silent self-update would be a way
/// to hand someone an unexamined binary; the browser shows them what they are
/// getting and where it came from.
#[tauri::command]
pub fn update_open(app: tauri::AppHandle, url: String) -> Result<(), String> {
    // Checked here as well as when the asset was chosen: this is the call that
    // actually acts on the URL, so this is where the guard has to hold.
    if !update::is_trusted_download(&url) {
        return Err("that download is not on GitHub — refusing to open it".into());
    }
    app.opener()
        .open_url(url, None::<&str>)
        .map_err(|e| format!("could not open the browser: {e}"))
}
