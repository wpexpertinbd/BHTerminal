import SwiftUI
import UniformTypeIdentifiers

/// Add/edit form for a saved Host. Secrets (password / key passphrase) are
/// read from and written to the Keychain directly — never held in the
/// SessionStore's JSON alongside the rest of the host's fields.
struct HostEditorView: View {
    @Environment(\.dismiss) private var dismiss

    let store: SessionStore
    let editingHost: Host?
    private let initialFolderID: UUID?

    /// Picker selection is kept separate from the key path so tag equality
    /// never has to compare associated values — Host.AuthMethod is only
    /// assembled from these two at save time.
    private enum AuthKind: Hashable { case agent, password, privateKey }

    @State private var name: String
    @State private var hostname: String
    @State private var port: String
    @State private var username: String
    @State private var connectionType: Host.ConnectionType
    @State private var authKind: AuthKind
    @State private var keyPath: String
    @State private var password: String = ""
    @State private var passphrase: String = ""
    @State private var jumpHostID: UUID?
    @State private var folderID: UUID?
    @State private var notes: String
    @State private var vncSharedClipboard: Bool
    @State private var isChoosingKeyFile = false

    init(store: SessionStore, folderID: UUID? = nil, editingHost: Host? = nil) {
        self.store = store
        self.initialFolderID = folderID
        self.editingHost = editingHost

        _name = State(initialValue: editingHost?.name ?? "")
        _hostname = State(initialValue: editingHost?.hostname ?? "")
        _port = State(initialValue: String(editingHost?.port ?? 22))
        _username = State(initialValue: editingHost?.username ?? NSUserName())
        _connectionType = State(initialValue: editingHost?.connectionType ?? .ssh)
        _vncSharedClipboard = State(initialValue: editingHost?.vncSharedClipboard ?? false)

        switch editingHost?.authMethod {
        case .password: _authKind = State(initialValue: .password)
        case .privateKey: _authKind = State(initialValue: .privateKey)
        case .agent, nil: _authKind = State(initialValue: .agent)
        }

        if case .privateKey(let path) = editingHost?.authMethod {
            _keyPath = State(initialValue: path)
        } else {
            _keyPath = State(initialValue: "~/.ssh/id_ed25519")
        }

        _jumpHostID = State(initialValue: editingHost?.jumpHostID)
        _folderID = State(initialValue: editingHost?.folderID ?? folderID)
        _notes = State(initialValue: editingHost?.notes ?? "")
    }

    private var isEditing: Bool { editingHost != nil }

    private var candidateJumpHosts: [Host] {
        store.hosts.filter { $0.id != editingHost?.id }
    }

    private var folderOptions: [(id: UUID?, label: String)] {
        var options: [(id: UUID?, label: String)] = [(nil, "No Folder")]
        func walk(_ parentID: UUID?, depth: Int) {
            let children = store.folders
                .filter { $0.parentFolderID == parentID }
                .sorted { $0.sortOrder < $1.sortOrder }
            for folder in children {
                options.append((folder.id, String(repeating: "  ", count: depth) + folder.name))
                walk(folder.id, depth: depth + 1)
            }
        }
        walk(nil, depth: 0)
        return options
    }

    var body: some View {
        Form {
            Section("Connection") {
                Picker("Type", selection: $connectionType) {
                    Text("SSH").tag(Host.ConnectionType.ssh)
                    Text("VNC").tag(Host.ConnectionType.vnc)
                }
                .pickerStyle(.segmented)
                .onChange(of: connectionType) { _, newValue in
                    // Nudge toward each type's default port, but only if
                    // it's still sitting at the OTHER type's default —
                    // leaves a deliberately-customized port alone.
                    if newValue == .vnc, port == "22" { port = "5900" }
                    if newValue == .ssh, port == "5900" { port = "22" }
                }

                TextField("Name", text: $name, prompt: Text("My Server"))
                TextField("Hostname", text: $hostname, prompt: Text("example.com"))
                TextField("Port", text: $port)
                    .frame(maxWidth: 100)
                TextField("Username", text: $username)
                Picker("Folder", selection: $folderID) {
                    ForEach(folderOptions, id: \.id) { option in
                        Text(option.label).tag(option.id)
                    }
                }
            }

            if connectionType == .ssh {
                Section("Authentication") {
                    Picker("Method", selection: $authKind) {
                        Text("SSH Agent").tag(AuthKind.agent)
                        Text("Password").tag(AuthKind.password)
                        Text("Private Key").tag(AuthKind.privateKey)
                    }
                    .pickerStyle(.segmented)

                    switch authKind {
                    case .password:
                        SecureField("Password", text: $password)
                    case .privateKey:
                        HStack {
                            TextField("Key path", text: $keyPath)
                            Button("Choose…") { isChoosingKeyFile = true }
                        }
                        SecureField("Passphrase (optional)", text: $passphrase)
                    case .agent:
                        Text("Uses ssh-agent / keys already loaded for your user account.")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                }

                Section("Jump Host") {
                    Picker("Via", selection: $jumpHostID) {
                        Text("None").tag(UUID?.none)
                        ForEach(candidateJumpHosts) { host in
                            Text(host.name).tag(Optional(host.id))
                        }
                    }
                }
            } else {
                Section("Password") {
                    SecureField("Password", text: $password)
                    Text("Standard VNC auth is password-only. Apple Remote Desktop / UltraVNC MS-Logon also use the username above. To tunnel VNC through SSH, add a local port forward on an SSH host (via its Tunnels menu) and point this host at 127.0.0.1 + the forwarded port.")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
                Section("Options") {
                    Toggle("Share clipboard with remote", isOn: $vncSharedClipboard)
                    Text("Off by default. When on, this VNC server can read and change your local clipboard — only enable it for servers you fully trust.")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            }

            Section("Notes") {
                TextEditor(text: $notes)
                    .frame(minHeight: 60)
            }

            if let validationError {
                Section {
                    Label(validationError, systemImage: "exclamationmark.triangle.fill")
                        .foregroundStyle(.orange)
                        .font(.caption)
                }
            }
        }
        .formStyle(.grouped)
        .frame(minWidth: 420, minHeight: 480)
        .onAppear(perform: loadSecrets)
        // SwiftUI-owned file picker — see applyChosenKey for why this must not
        // be a hand-rolled NSOpenPanel.
        .fileImporter(isPresented: $isChoosingKeyFile,
                      allowedContentTypes: [.item],
                      onCompletion: applyChosenKey)
        // Keys live in ~/.ssh, which is hidden.
        .fileDialogDefaultDirectory(
            FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent(".ssh"))
        .fileDialogBrowserOptions(.includeHiddenFiles)
        .safeAreaInset(edge: .bottom) {
            HStack {
                Spacer()
                Button("Cancel", role: .cancel) { dismiss() }
                Button(isEditing ? "Save" : "Add") { save() }
                    .keyboardShortcut(.defaultAction)
                    .disabled(!canSave)
            }
            .padding()
            .background(.bar)
        }
    }

    /// Rejects hostnames/usernames that ssh could misinterpret as options —
    /// the same allow-list SSHSafety enforces at connect time, surfaced here
    /// so a bad value can't be saved in the first place. Returns nil when OK.
    private var validationError: String? {
        let trimmedHost = hostname.trimmingCharacters(in: .whitespaces)
        if !trimmedHost.isEmpty && !SSHSafety.isValidHostname(trimmedHost) {
            return SSHSafetyError.unsafeHostname(trimmedHost).errorDescription
        }
        if connectionType == .ssh {
            let trimmedUser = username.trimmingCharacters(in: .whitespaces)
            if !trimmedUser.isEmpty && !SSHSafety.isValidUsername(trimmedUser) {
                return SSHSafetyError.unsafeUsername(trimmedUser).errorDescription
            }
        }
        return nil
    }

    private var canSave: Bool {
        !name.trimmingCharacters(in: .whitespaces).isEmpty
            && !hostname.trimmingCharacters(in: .whitespaces).isEmpty
            && (connectionType != .ssh || !username.trimmingCharacters(in: .whitespaces).isEmpty)
            && validationError == nil
    }

    private func loadSecrets() {
        guard let editingHost else { return }
        if editingHost.connectionType == .vnc || editingHost.authMethod == .password {
            password = (try? KeychainService.read(account: editingHost.keychainAccount)) ?? ""
        }
        if case .privateKey = editingHost.authMethod {
            passphrase = (try? KeychainService.read(account: editingHost.passphraseAccount)) ?? ""
        }
    }

    /// The key picker MUST NOT be a hand-rolled NSOpenPanel.
    ///
    /// Running `NSOpenPanel.runModal()` from inside this view — which is itself
    /// presented as a SwiftUI `.sheet` — leaves the panel's out-of-process
    /// `NSRemoteView` (ViewBridge) attached to the sheet's window. Presenting
    /// the sheet again a few hosts later made that stale remote view throw from
    /// `-[NSRemoteView containingWindowWillOrderOnScreen:]`, inside SwiftUI's
    /// SheetBridge. AppKit swallowed the exception, so the app kept running with
    /// its sidebar view graph broken: no sheets, no delete confirmation, no
    /// working "+" menu and no folder expand/collapse until relaunch.
    ///
    /// `.fileImporter` is SwiftUI's own presentation, coordinated with the sheet
    /// machinery instead of fighting it. (Proven by the crash report from
    /// v1.2.2 with NSApplicationCrashOnExceptions enabled.)
    private func applyChosenKey(_ result: Result<URL, Error>) {
        if case .success(let url) = result {
            keyPath = url.path
        }
    }

    private func save() {
        // Belt-and-suspenders: the Save button is already disabled while
        // validationError != nil, but never persist an unsafe host.
        guard validationError == nil else { return }

        var host = editingHost ?? Host(name: name, hostname: hostname, username: username, folderID: initialFolderID)
        host.name = name
        host.hostname = hostname
        host.port = Int(port) ?? (connectionType == .vnc ? 5900 : 22)
        host.username = username
        host.connectionType = connectionType
        host.vncSharedClipboard = vncSharedClipboard

        switch connectionType {
        case .ssh:
            switch authKind {
            case .agent: host.authMethod = .agent
            case .password: host.authMethod = .password
            case .privateKey: host.authMethod = .privateKey(path: keyPath)
            }
            host.jumpHostID = jumpHostID
        case .vnc:
            host.authMethod = .password
            host.jumpHostID = nil
        }
        host.folderID = folderID
        host.notes = notes

        if editingHost != nil {
            store.updateHost(host)
        } else {
            host = store.addHost(host)
        }

        // The fields are pre-filled from the Keychain when editing, so an empty
        // one means the user deliberately cleared it — delete the stored secret
        // rather than leaving the old value behind (clearing a passphrase used
        // to silently keep it forever).
        switch connectionType {
        case .ssh:
            switch authKind {
            case .password:
                saveOrClear(secret: password, account: host.keychainAccount)
                try? KeychainService.delete(account: host.passphraseAccount)
            case .privateKey:
                saveOrClear(secret: passphrase, account: host.passphraseAccount)
                try? KeychainService.delete(account: host.keychainAccount)
            case .agent:
                // Agent auth needs neither, so don't leave stale secrets around.
                try? KeychainService.delete(account: host.keychainAccount)
                try? KeychainService.delete(account: host.passphraseAccount)
            }
        case .vnc:
            saveOrClear(secret: password, account: host.keychainAccount)
        }

        dismiss()
    }

    private func saveOrClear(secret: String, account: String) {
        if secret.isEmpty {
            try? KeychainService.delete(account: account)
        } else {
            try? KeychainService.save(account: account, secret: secret)
        }
    }
}
