import SwiftUI

/// Small reusable "type a name, Save/Cancel" sheet — used for folder
/// create/rename in the session sidebar and for SFTP new-folder/rename.
struct TextPromptSheet: View {
    @Environment(\.dismiss) private var dismiss
    let title: String
    var initialValue: String = ""
    var placeholder: String = ""
    let onSave: (String) -> Void

    @State private var text: String = ""

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text(title).font(.headline)
            TextField(placeholder, text: $text)
                .textFieldStyle(.roundedBorder)
                .onSubmit(save)
            HStack {
                Spacer()
                Button("Cancel", role: .cancel) { dismiss() }
                Button("Save") { save() }
                    .keyboardShortcut(.defaultAction)
                    .disabled(text.trimmingCharacters(in: .whitespaces).isEmpty)
            }
        }
        .padding()
        .frame(width: 320)
        .onAppear { text = initialValue }
    }

    private func save() {
        let trimmed = text.trimmingCharacters(in: .whitespaces)
        guard !trimmed.isEmpty else { return }
        onSave(trimmed)
        dismiss()
    }
}
