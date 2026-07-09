import SwiftUI

/// Cmd+, preferences window — general (launch at login) + appearance.
/// Appearance changes apply to newly-opened tabs/panes (see AppTheme's note
/// on why this isn't retroactive).
struct PreferencesView: View {
    @Bindable var settings = AppSettings.shared

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
        }
        .formStyle(.grouped)
        .frame(width: 420)
        .padding()
        // Reflect changes made outside the app (e.g. System Settings > Login Items).
        .onAppear { launchAtLogin = LoginItem.isEnabled }
    }
}
