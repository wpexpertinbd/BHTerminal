import SwiftUI

/// Cmd+, preferences window — general (launch at login) + appearance.
/// Appearance changes apply to newly-opened tabs/panes (see AppTheme's note
/// on why this isn't retroactive).
struct PreferencesView: View {
    @Bindable var settings = AppSettings.shared
    var updater = UpdateChecker.shared

    @State private var launchAtLogin = LoginItem.isEnabled
    @State private var loginItemError: String?

    var body: some View {
        Form {
            Section("General") {
                Toggle("Launch BHTerminal at login", isOn: $launchAtLogin)
                    .onChange(of: launchAtLogin) { _, newValue in
                        do {
                            try LoginItem.setEnabled(newValue)
                            loginItemError = nil
                        } catch {
                            // Revert the toggle to the OS's real state.
                            launchAtLogin = LoginItem.isEnabled
                            loginItemError = error.localizedDescription
                        }
                    }
                Text("Closing the window keeps BHTerminal running in the menu bar (top-right); tunnels keep forwarding. Use the menu-bar icon to reopen or quit.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                if let loginItemError {
                    Label(loginItemError, systemImage: "exclamationmark.triangle.fill")
                        .font(.caption)
                        .foregroundStyle(.orange)
                }
            }

            Section("Appearance") {
                Picker("Theme", selection: $settings.theme) {
                    ForEach(AppTheme.allCases) { theme in
                        Text(theme.displayName).tag(theme)
                    }
                }

                Picker("Font", selection: $settings.fontName) {
                    ForEach(AppSettings.monospaceFontNames, id: \.self) { name in
                        Text(name).tag(name)
                    }
                }

                Stepper("Size: \(Int(settings.fontSize))pt", value: $settings.fontSize, in: 9...24)

                Text("New terminal tabs and panes pick up these settings; already-open ones keep what they started with.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }

            updatesSection
        }
        .formStyle(.grouped)
        .frame(width: 420)
        .padding()
        // Reflect changes made outside the app (e.g. System Settings > Login Items).
        .onAppear { launchAtLogin = LoginItem.isEnabled }
    }

    // MARK: - Updates

    @ViewBuilder
    private var updatesSection: some View {
        Section("Updates") {
            Toggle("Check for updates automatically", isOn: Binding(
                get: { updater.autoCheckEnabled },
                set: { on in
                    updater.autoCheckEnabled = on
                    // Re-check right away when switching it on.
                    if on { Task { await updater.check() } }
                }
            ))

            HStack {
                Text("Version \(updater.currentVersion)")
                    .foregroundStyle(.secondary)
                Spacer()
                Button("Check Now") { Task { await updater.check() } }
                    .disabled(isBusy)
            }

            switch updater.status {
            case .checking:
                HStack(spacing: 6) {
                    ProgressView().controlSize(.small)
                    Text("Checking…").font(.caption).foregroundStyle(.secondary)
                }
            case .working:
                HStack(spacing: 6) {
                    ProgressView().controlSize(.small)
                    Text("Downloading update…").font(.caption).foregroundStyle(.secondary)
                }
            case .upToDate:
                Label("You're on the latest version.", systemImage: "checkmark.circle.fill")
                    .font(.caption)
                    .foregroundStyle(.green)
            case .available(let version, _):
                VStack(alignment: .leading, spacing: 6) {
                    Label("BHTerminal \(version) is available.", systemImage: "arrow.down.circle.fill")
                        .font(.caption)
                        .foregroundStyle(Color.accentColor)
                    HStack {
                        Button("Update Now") { Task { await updater.downloadAndInstall() } }
                            .buttonStyle(.borderedProminent)
                            .controlSize(.small)
                        Button("Release Notes") { updater.openReleasesPage() }
                            .controlSize(.small)
                    }
                    Text("BHTerminal quits so the installer can replace it, then you reopen it.")
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                }
            case .failed(let message):
                VStack(alignment: .leading, spacing: 6) {
                    Label(message, systemImage: "exclamationmark.triangle.fill")
                        .font(.caption)
                        .foregroundStyle(.orange)
                    Button("Try Again") { Task { await updater.check() } }
                        .controlSize(.small)
                }
            case .idle:
                EmptyView()
            }
        }
    }

    private var isBusy: Bool {
        updater.status == .checking || updater.status == .working
    }
}
