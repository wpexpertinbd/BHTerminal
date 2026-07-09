import SwiftUI

/// Cmd+, preferences window — theme + font. Changes apply to newly-opened
/// tabs/panes (see AppTheme's note on why this isn't retroactive).
struct PreferencesView: View {
    @Bindable var settings = AppSettings.shared

    var body: some View {
        Form {
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
        .formStyle(.grouped)
        .frame(width: 380)
        .padding()
    }
}
