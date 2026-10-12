import { useEffect, useState } from "react";
import { open as openFile, save as saveFile } from "@tauri-apps/plugin-dialog";
import { api } from "./api";

type Mode = "export" | "restore" | "import";

interface Props {
  mode: Mode;
  /** Called after anything that changed the store, so the tree reloads. */
  onChanged: () => void;
  onClose: () => void;
}

const TITLES: Record<Mode, string> = {
  export: "Export a backup",
  restore: "Restore from a backup",
  import: "Import sessions",
};

export function TransferSheet({ mode, onChanged, onClose }: Props) {
  const [includeSecrets, setIncludeSecrets] = useState(false);
  const [passphrase, setPassphrase] = useState("");
  const [confirm, setConfirm] = useState("");
  const [needsPassphrase, setNeedsPassphrase] = useState<string | null>(null);
  const [pending, setPending] = useState<{ path: string; hosts: number; source: string } | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [done, setDone] = useState<string[] | null>(null);
  const [warnings, setWarnings] = useState<string[]>([]);
  /** null until known: the export form must not guess, or a slow check
   *  shows the passphrase form and the backend then refuses for a master
   *  password the user was never asked for. */
  const [hasMaster, setHasMaster] = useState<boolean | null>(null);
  const [master, setMaster] = useState("");
  /** Like MobaXterm: by default the archive is locked with the master
   *  password itself, so there is one password to remember, not two. */
  const [useMaster, setUseMaster] = useState(true);

  useEffect(() => {
    api
      .vaultStatus()
      .then((s) => setHasMaster(s.hasMaster))
      .catch((e) => setError(`Couldn't check the master password: ${e}`));
  }, []);
  const masterLocksFile = hasMaster === true && useMaster;

  async function runExport() {
    if (includeSecrets && hasMaster && !master) {
      return setError("Enter your master password to export saved passwords.");
    }
    if (includeSecrets && !masterLocksFile) {
      // Secrets force encryption, so the passphrase is the archive's only
      // protection — a typo in it makes the file permanently unreadable.
      if (passphrase.length < 8) return setError("Use a passphrase of at least 8 characters.");
      if (passphrase !== confirm) return setError("The two passphrases do not match.");
    }
    const path = await saveFile({
      title: "Save backup",
      defaultPath: `BHTerminal-backup-${new Date().toISOString().slice(0, 10)}.bhterminal.json`,
    });
    if (!path) return;
    setBusy(true);
    setError(null);
    try {
      const filePassword = masterLocksFile ? master : passphrase;
      const secrets = await api.backupExport(
        path,
        includeSecrets,
        includeSecrets ? filePassword : null,
        includeSecrets && hasMaster ? master : null
      );
      // The passwords have done their job; don't keep them in memory.
      setMaster("");
      setPassphrase("");
      setConfirm("");
      setDone([
        `Saved to ${path}`,
        includeSecrets
          ? `${secrets} saved password${secrets === 1 ? "" : "s"} included, encrypted with ${masterLocksFile ? "your master password" : "the passphrase you chose"}. There is no way to open the file without it.`
          : "Hosts, folders and snippets only — no passwords. The file is plain readable JSON.",
      ]);
    } catch (e) {
      setError(String(e));
    }
    setBusy(false);
  }

  async function pickRestore() {
    const picked = await openFile({ multiple: false, title: "Choose a backup" });
    if (typeof picked !== "string") return;
    setBusy(true);
    setError(null);
    try {
      if (await api.backupIsEncrypted(picked)) {
        setNeedsPassphrase(picked);
      } else {
        await doRestore(picked, null);
      }
    } catch (e) {
      setError(String(e));
    }
    setBusy(false);
  }

  async function doRestore(path: string, pass: string | null) {
    const s = await api.backupRestore(path, pass);
    onChanged();
    setNeedsPassphrase(null);
    setDone([
      `${s.hostsAdded} host${s.hostsAdded === 1 ? "" : "s"} added, ${s.hostsUpdated} updated.`,
      `${s.foldersAdded} folder${s.foldersAdded === 1 ? "" : "s"} added, ${s.snippetsAdded} snippet${s.snippetsAdded === 1 ? "" : "s"} added.`,
      s.secretsRestored > 0
        ? `${s.secretsRestored} saved password${s.secretsRestored === 1 ? "" : "s"} restored.`
        : "No passwords were in this backup.",
      "Nothing was deleted — a restore only adds and updates.",
    ]);
    setWarnings(
      s.missingKeys.length > 0
        ? [
            `${s.missingKeys.length} host${s.missingKeys.length === 1 ? "" : "s"} use a private key file that is not on this computer. Copy the key over, or open the host and choose the key again:`,
            ...s.missingKeys.slice(0, 10),
            ...(s.missingKeys.length > 10 ? [`…and ${s.missingKeys.length - 10} more.`] : []),
          ]
        : []
    );
  }

  async function pickImport() {
    const picked = await openFile({
      multiple: false,
      title: "Choose a MobaXterm export or an SSH config",
    });
    if (typeof picked !== "string") return;
    setBusy(true);
    setError(null);
    try {
      const preview = await api.importPreview(picked);
      setWarnings(preview.warnings);
      setPending({ path: picked, hosts: preview.hosts, source: preview.source });
    } catch (e) {
      setError(String(e));
    }
    setBusy(false);
  }

  async function applyImport() {
    if (!pending) return;
    setBusy(true);
    setError(null);
    try {
      const r = await api.importApply(pending.path);
      onChanged();
      setPending(null);
      setWarnings(r.warnings);
      setDone([
        `${r.hostsAdded} host${r.hostsAdded === 1 ? "" : "s"} imported into “${r.source} Import”.`,
        r.passwordsSaved > 0 ? `${r.passwordsSaved} password${r.passwordsSaved === 1 ? "" : "s"} stored.` : "",
        r.skipped > 0 ? `${r.skipped} skipped — see below.` : "",
      ].filter(Boolean));
    } catch (e) {
      setError(String(e));
    }
    setBusy(false);
  }

  return (
    <div className="modal-backdrop" onClick={busy ? undefined : onClose}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <h2>{TITLES[mode]}</h2>

        {done ? (
          <>
            {done.map((line, i) => (
              <p key={i} className="done-line">
                {line}
              </p>
            ))}
            {warnings.length > 0 && (
              <div className="warnings">
                {warnings.map((w, i) => (
                  <p key={i}>{w}</p>
                ))}
              </div>
            )}
          </>
        ) : mode === "export" ? (
          <>
            <label className="check">
              <input
                type="checkbox"
                checked={includeSecrets}
                disabled={hasMaster === null}
                onChange={(e) => setIncludeSecrets(e.target.checked)}
              />
              <span>Include saved passwords and key passphrases</span>
            </label>
            {includeSecrets && hasMaster && (
              <>
                <label className="field">
                  <span>Your master password</span>
                  <input type="password" autoComplete="current-password" value={master} autoFocus onChange={(e) => setMaster(e.target.value)} />
                </label>
                <label className="check">
                  <input type="checkbox" checked={useMaster} onChange={(e) => setUseMaster(e.target.checked)} />
                  <span>Encrypt the file with my master password</span>
                </label>
                {useMaster && (
                  <p className="hint">
                    Restoring it — here or on another computer — will ask for this master password.
                  </p>
                )}
              </>
            )}
            {includeSecrets && masterLocksFile ? null : includeSecrets ? (
              <>
                <label className="field">
                  <span>Passphrase for the archive</span>
                  <input type="password" autoComplete="new-password" value={passphrase} onChange={(e) => setPassphrase(e.target.value)} />
                </label>
                <label className="field">
                  <span>Again</span>
                  <input type="password" autoComplete="new-password" value={confirm} onChange={(e) => setConfirm(e.target.value)} />
                  <small>
                    Secrets are only ever written encrypted. Nobody — including BHTerminal — can open
                    the file without this passphrase.
                  </small>
                </label>
              </>
            ) : (
              <p className="done-line">
                The backup will hold your hosts, folders and snippets as readable JSON. Passwords stay
                on this computer.
              </p>
            )}
          </>
        ) : mode === "restore" ? (
          needsPassphrase ? (
            <label className="field">
              <span>This backup is encrypted. Enter its password — the master password it was exported with, or the passphrase chosen then.</span>
              <input
                type="password"
                value={passphrase}
                autoFocus
                onChange={(e) => setPassphrase(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter") {
                    setBusy(true);
                    doRestore(needsPassphrase, passphrase)
                      .catch((err) => setError(String(err)))
                      .finally(() => setBusy(false));
                  }
                }}
              />
            </label>
          ) : (
            <p className="done-line">
              Choose a <code>.bhterminal.json</code> backup. Hosts and folders are merged by id —
              nothing already here is deleted.
            </p>
          )
        ) : pending ? (
          <>
            <p className="done-line">
              Found <strong>{pending.hosts}</strong> session{pending.hosts === 1 ? "" : "s"} in this{" "}
              {pending.source} file. They will go into a new folder called “{pending.source} Import”,
              keeping the folder structure from the export.
            </p>
            {warnings.length > 0 && (
              <div className="warnings">
                {warnings.slice(0, 8).map((w, i) => (
                  <p key={i}>{w}</p>
                ))}
                {warnings.length > 8 && <p>…and {warnings.length - 8} more.</p>}
              </div>
            )}
          </>
        ) : (
          <>
            <p className="done-line">BHTerminal reads:</p>
            <ul className="bullets">
              <li>
                <strong>MobaXterm sessions</strong> — <code>.mxtsessions</code>, a plain{" "}
                <code>.mobaconf</code> or <code>MobaXterm.ini</code>. No passwords in this format.
              </li>
              <li>
                <strong>MobaXterm stored passwords</strong> — Settings → Configuration → General →
                “passwords management” → “Export to file”. This one carries the passwords, and they go
                straight into the vault.
              </li>
              <li>
                <strong>An OpenSSH config</strong> — <code>~/.ssh/config</code>.
              </li>
            </ul>
          </>
        )}

        {error && <div className="error">{error}</div>}

        <div className="modal-actions">
          <button onClick={onClose} disabled={busy}>
            {done ? "Close" : "Cancel"}
          </button>
          {!done && mode === "export" && (
            <button className="primary" onClick={runExport} disabled={busy}>
              {busy ? "Saving…" : "Choose a location…"}
            </button>
          )}
          {!done && mode === "restore" && !needsPassphrase && (
            <button className="primary" onClick={pickRestore} disabled={busy}>
              {busy ? "Restoring…" : "Choose a file…"}
            </button>
          )}
          {!done && mode === "restore" && needsPassphrase && (
            <button
              className="primary"
              disabled={busy || !passphrase}
              onClick={() => {
                setBusy(true);
                doRestore(needsPassphrase, passphrase)
                  .catch((e) => setError(String(e)))
                  .finally(() => setBusy(false));
              }}
            >
              {busy ? "Restoring…" : "Unlock and restore"}
            </button>
          )}
          {!done && mode === "import" && !pending && (
            <button className="primary" onClick={pickImport} disabled={busy}>
              {busy ? "Reading…" : "Choose a file…"}
            </button>
          )}
          {!done && mode === "import" && pending && (
            <button className="primary" onClick={applyImport} disabled={busy}>
              {busy ? "Importing…" : `Import ${pending.hosts}`}
            </button>
          )}
        </div>
      </div>
    </div>
  );
}
