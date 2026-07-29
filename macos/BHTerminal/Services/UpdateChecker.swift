import AppKit
import Foundation

/// Checks GitHub Releases for a newer BHTerminal and installs it.
///
/// Same approach as BHServe: read the latest release from the public API,
/// compare it against this build's version, then hand the release's .pkg to
/// the macOS Installer and quit so it can replace the running app. Nothing is
/// installed without the user asking for it.
@MainActor
@Observable
final class UpdateChecker {
    static let shared = UpdateChecker()

    static let repoSlug = "wpexpertinbd/BHTerminal"
    static let releasesPage = "https://github.com/\(repoSlug)/releases/latest"

    enum Status: Equatable {
        case idle, checking, working, upToDate
        case available(version: String, pkg: String)
        case failed(String)
    }

    private(set) var status: Status = .idle

    /// True only when a newer version was found — drives the menu-bar hint.
    var updateAvailable: Bool {
        if case .available = status { return true }
        return false
    }

    var availableVersion: String? {
        if case .available(let version, _) = status { return version }
        return nil
    }

    /// This build's version, from the bundle.
    var currentVersion: String {
        Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? "0"
    }

    // MARK: - Preference

    private static let autoCheckKey = "autoUpdateCheck"

    /// "Check for updates automatically" — default ON, persisted.
    var autoCheckEnabled: Bool = (UserDefaults.standard.object(forKey: UpdateChecker.autoCheckKey) as? Bool) ?? true {
        didSet { UserDefaults.standard.set(autoCheckEnabled, forKey: UpdateChecker.autoCheckKey) }
    }

    /// When an automatic check last actually hit GitHub. Persisted so restarts
    /// don't each fire a request.
    private var lastAutoCheck: Date {
        get { Date(timeIntervalSince1970: UserDefaults.standard.double(forKey: "lastUpdateCheckAt")) }
        set { UserDefaults.standard.set(newValue.timeIntervalSince1970, forKey: "lastUpdateCheckAt") }
    }
    private let autoCheckMinInterval: TimeInterval = 30 * 60

    private init() {}

    // MARK: - Checking

    private struct Release: Decodable {
        let tagName: String
        let htmlURL: String
        let assets: [Asset]

        struct Asset: Decodable {
            let name: String
            let url: String
            enum CodingKeys: String, CodingKey { case name; case url = "browser_download_url" }
        }
        enum CodingKeys: String, CodingKey {
            case tagName = "tag_name"
            case htmlURL = "html_url"
            case assets
        }
    }

    /// `auto` = the quiet launch/background check: it never shows a spinner or a
    /// "check failed" (being offline at login is normal) — it only promotes to
    /// `.available`. A manual check always runs and always reports.
    func check(auto: Bool = false) async {
        // GitHub's unauthenticated API allows only 60 requests/hour/IP, shared
        // across a whole network, so automatic checks are throttled. Stamp the
        // time up front so a rate-limit response backs off too.
        if auto {
            guard autoCheckEnabled else { return }
            if Date().timeIntervalSince(lastAutoCheck) < autoCheckMinInterval { return }
            lastAutoCheck = Date()
        }
        if !auto { status = .checking }

        guard let url = URL(string: "https://api.github.com/repos/\(Self.repoSlug)/releases/latest") else {
            if !auto { status = .failed("Bad update URL") }
            return
        }

        do {
            var request = URLRequest(url: url)
            request.setValue("application/vnd.github+json", forHTTPHeaderField: "Accept")
            request.setValue("BHTerminal/\(currentVersion)", forHTTPHeaderField: "User-Agent")
            request.timeoutInterval = 20

            let (data, response) = try await URLSession.shared.data(for: request)
            let code = (response as? HTTPURLResponse)?.statusCode ?? 0
            guard code == 200 else {
                // Never claim "up to date" after a failed check — that hides
                // the real state.
                if !auto {
                    status = .failed(code == 403
                        ? "GitHub rate-limited the check — try again in a few minutes."
                        : "Couldn't reach GitHub (HTTP \(code)).")
                }
                return
            }

            let release = try JSONDecoder().decode(Release.self, from: data)
            let latest = release.tagName.trimmingCharacters(in: CharacterSet(charactersIn: "vV "))
            guard Self.isNewer(latest, than: currentVersion) else {
                if !auto { status = .upToDate }
                return
            }
            // Only ever install from the release's own .pkg asset.
            guard let pkg = release.assets.first(where: { $0.name.hasSuffix(".pkg") })?.url,
                  Self.isTrustedAssetURL(pkg) else {
                if !auto { status = .failed("Release \(latest) has no installer to download.") }
                return
            }
            status = .available(version: latest, pkg: pkg)
        } catch {
            if !auto { status = .failed(error.localizedDescription) }
        }
    }

    /// Numeric version compare ("1.10.0" > "1.9.3"). Pure — no actor state.
    nonisolated static func isNewer(_ candidate: String, than current: String) -> Bool {
        let a = candidate.split(separator: ".").map { Int($0) ?? 0 }
        let b = current.split(separator: ".").map { Int($0) ?? 0 }
        for i in 0..<max(a.count, b.count) {
            let x = i < a.count ? a[i] : 0
            let y = i < b.count ? b[i] : 0
            if x != y { return x > y }
        }
        return false
    }

    /// The installer must come from GitHub's own release host over HTTPS — a
    /// tampered API response mustn't be able to point the updater at an
    /// arbitrary download.
    nonisolated static func isTrustedAssetURL(_ raw: String) -> Bool {
        guard let url = URL(string: raw),
              url.scheme?.lowercased() == "https",
              let host = url.host?.lowercased() else { return false }
        return host == "github.com" || host == "objects.githubusercontent.com"
            || host.hasSuffix(".github.com") || host.hasSuffix(".githubusercontent.com")
    }

    // MARK: - Installing

    /// Downloads the .pkg and opens it in the macOS Installer, then quits so the
    /// running app can be replaced (the installer's preinstall step also stops
    /// any copy that's still running).
    func downloadAndInstall() async {
        guard case .available(_, let pkg) = status else { return }
        guard Self.isTrustedAssetURL(pkg), let url = URL(string: pkg) else {
            status = .failed("Update came from an unexpected location — cancelled.")
            return
        }
        status = .working
        do {
            let (tmp, response) = try await URLSession.shared.download(from: url)
            let code = (response as? HTTPURLResponse)?.statusCode ?? 0
            guard code == 200 else {
                status = .failed("Download failed (HTTP \(code)).")
                return
            }
            let dest = FileManager.default.temporaryDirectory
                .appendingPathComponent("BHTerminal-update.pkg")
            try? FileManager.default.removeItem(at: dest)
            try FileManager.default.moveItem(at: tmp, to: dest)
            NSWorkspace.shared.open(dest)
            try? await Task.sleep(for: .seconds(1))
            NSApp.terminate(nil)
        } catch {
            status = .failed(error.localizedDescription)
        }
    }

    func openReleasesPage() {
        if let url = URL(string: Self.releasesPage) {
            NSWorkspace.shared.open(url)
        }
    }
}
