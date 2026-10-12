import { useEffect, useState } from "react";
import { open as openFile } from "@tauri-apps/plugin-dialog";
import { api, authKind, type AuthMethod, type Host, type HostFolder } from "./api";

interface Props {
  host: Host;
  /** New hosts have nothing stored yet, so no round-trip is needed for them. */
  isNew: boolean;
  folders: HostFolder[];
  /** Candidate jump hosts: every other SSH host. */
  hosts: Host[];
  onSaved: (host: Host) => void;
  onCancel: () => void;
}

/** A VNC server's default display; SSH's is 22. Changing the connection type
 *  moves the port with it unless the user has typed their own. */
const DEFAULT_PORT = { ssh: 22, vnc: 5900 } as const;

export function HostEditor({ host, isNew, folders, hosts, onSaved, onCancel }: Props) {
  const [draft, setDraft] = useState<Host>(host);
  const [secret, setSecret] = useState("");
  const [stored, setStored] = useState(false);
  const [clearSecret, setClearSecret] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (isNew) return;
    api.hostHasSecret(host.id).then(setStored).catch(() => {});
  }, [host.id, isNew]);

  const kind = authKind(draft.authMethod);
  const isVnc = draft.connectionType === "vnc";

  function set<K extends keyof Host>(key: K, value: Host[K]) {
    setDraft((d) => ({ ...d, [key]: value }));
  }

  function setType(type: "ssh" | "vnc") {
    setDraft((d) => ({
      ...d,
      connectionType: type,
      // Only replace a port that is still the other type's default, so a
      // hand-typed port is never silently overwritten.
      port: d.port === DEFAULT_PORT[d.connectionType] ? DEFAULT_PORT[type] : d.port,
    }));
  }

  function setAuth(next: "password" | "privateKey" | "agent") {
    const method: AuthMethod =
      next === "password"
        ? { password: {} }
        : next === "agent"
          ? { agent: {} }
          : { privateKey: { path: kind === "privateKey" ? (draft.authMethod as { privateKey: { path: string } }).privateKey.path : "" } };
    set("authMethod", method);
    setSecret("");
    setClearSecret(false);
  }

  async function browseKey() {
    const picked = await openFile({
      multiple: false,
      directory: false,
      title: "Choose a private key",
    });
    if (typeof picked === "string") set("authMethod", { privateKey: { path: picked } });
  }

  async function save() {
    const name = draft.name.trim() || draft.hostname.trim();
    const hostname = draft.hostname.trim();
    if (!hostname) return setError("A hostname or IP address is required.");
    if (!isVnc && !draft.username.trim()) return setError("A username is required for SSH.");
    if (kind === "privateKey" && !(draft.authMethod as { privateKey: { path: string } }).privateKey.path) {
      return setError("Choose a private key file, or switch to password or agent.");
    }
    setBusy(true);
    setError(null);
    try {
      // null = leave any stored secret alone; "" = delete it.
      const payload = secret ? secret : clearSecret ? "" : null;
      const saved = await api.saveHost({ ...draft, name, hostname }, payload);
      onSaved(saved);
    } catch (e) {
      setError(String(e));
      setBusy(false);
    }
  }

  const secretLabel = kind === "password" ? "Password" : "Key passphrase";

  return (
    <div className="modal-backdrop" onClick={onCancel}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <h2>{isNew ? "New host" : "Edit host"}</h2>

        <label className="field">
          <span>What are you connecting to?</span>
          <div className="seg">
            <button className={!isVnc ? "on" : ""} onClick={() => setType("ssh")}>
              SSH &amp; SFTP
            </button>
            <button className={isVnc ? "on" : ""} onClick={() => setType("vnc")}>
              VNC desktop
            </button>
          </div>
          <small>
            {isVnc
              ? "Opens the remote desktop in a tab."
              : "One tab gives you the shell and the file browser together — SFTP rides the same login."}
          </small>
        </label>

        <label className="field">
          <span>Name</span>
          <input
            value={draft.name}
            placeholder={draft.hostname || "my server"}
            onChange={(e) => set("name", e.target.value)}
            autoFocus
          />
        </label>

        <div className="row2">
          <label className="field">
            <span>Host or IP</span>
            <input
              value={draft.hostname}
              placeholder="203.0.113.10"
              onChange={(e) => set("hostname", e.target.value)}
            />
          </label>
          <label className="field narrow">
            <span>Port</span>
            <input
              type="number"
              min={1}
              max={65535}
              value={draft.port}
              onChange={(e) => set("port", Number(e.target.value) || DEFAULT_PORT[draft.connectionType])}
            />
          </label>
        </div>

        {!isVnc && (
          <label className="field">
            <span>Username</span>
            <input value={draft.username} placeholder="root" onChange={(e) => set("username", e.target.value)} />
          </label>
        )}

        {!isVnc && (
          <label className="field">
            <span>Sign in with</span>
            <div className="seg">
              <button className={kind === "password" ? "on" : ""} onClick={() => setAuth("password")}>
                Password
              </button>
              <button className={kind === "privateKey" ? "on" : ""} onClick={() => setAuth("privateKey")}>
                Private key
              </button>
              <button className={kind === "agent" ? "on" : ""} onClick={() => setAuth("agent")}>
                SSH agent
              </button>
            </div>
          </label>
        )}

        {kind === "privateKey" && !isVnc && (
          <label className="field">
            <span>Private key file</span>
            <div className="row2">
              <input
                value={(draft.authMethod as { privateKey: { path: string } }).privateKey.path}
                placeholder="~/.ssh/id_ed25519"
                onChange={(e) => set("authMethod", { privateKey: { path: e.target.value } })}
              />
              <button onClick={browseKey}>Choose…</button>
            </div>
          </label>
        )}

        {kind !== "agent" && (
          <label className="field">
            <span>{secretLabel}</span>
            <input
              type="password"
              value={secret}
              placeholder={stored && !clearSecret ? "•••••••• (saved — type to replace)" : "optional"}
              onChange={(e) => {
                setSecret(e.target.value);
                setClearSecret(false);
              }}
            />
            {stored && !secret && (
              <label className="check">
                <input
                  type="checkbox"
                  checked={clearSecret}
                  onChange={(e) => setClearSecret(e.target.checked)}
                />
                <span>Forget the saved {secretLabel.toLowerCase()}</span>
              </label>
            )}
            <small>
              Stored encrypted on this computer. Leave it empty and BHTerminal will let the
              server prompt you instead.
            </small>
          </label>
        )}

        <div className="row2">
          <label className="field">
            <span>Folder</span>
            <select
              value={draft.folderID ?? ""}
              onChange={(e) => set("folderID", e.target.value || null)}
            >
              <option value="">(none)</option>
              {folders.map((f) => (
                <option key={f.id} value={f.id}>
                  {f.name}
                </option>
              ))}
            </select>
          </label>
          {!isVnc && (
            <label className="field">
              <span>Jump host</span>
              <select
                value={draft.jumpHostID ?? ""}
                onChange={(e) => set("jumpHostID", e.target.value || null)}
              >
                <option value="">(direct)</option>
                {hosts
                  .filter((h) => h.id !== draft.id && h.connectionType === "ssh")
                  .map((h) => (
                    <option key={h.id} value={h.id}>
                      {h.name}
                    </option>
                  ))}
              </select>
            </label>
          )}
        </div>

        <label className="field">
          <span>Notes</span>
          <textarea rows={2} value={draft.notes} onChange={(e) => set("notes", e.target.value)} />
        </label>

        {error && <div className="error">{error}</div>}

        <div className="modal-actions">
          <button onClick={onCancel} disabled={busy}>
            Cancel
          </button>
          <button className="primary" onClick={save} disabled={busy}>
            {busy ? "Saving…" : "Save"}
          </button>
        </div>
      </div>
    </div>
  );
}
