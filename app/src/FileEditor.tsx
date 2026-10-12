import { useEffect, useRef, useState } from "react";
import { api } from "./api";

interface Props {
  hostId: string;
  /** Absolute path on the server. */
  remote: string;
  name: string;
  onClose: () => void;
  onSaved: () => void;
}

/** Edits one remote file.
 *
 *  The file is fetched to a private scratch copy, edited here, and uploaded
 *  back on save — the same trip an external editor makes, just without
 *  leaving the window. Nothing is written to the server until Save: closing
 *  with unsaved changes asks first, because the alternative is losing work
 *  silently. */
export function FileEditor({ hostId, remote, name, onClose, onSaved }: Props) {
  const [text, setText] = useState("");
  const [original, setOriginal] = useState("");
  const [status, setStatus] = useState("Opening…");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(true);
  const localRef = useRef<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const local = await api.editScratchPath(hostId, remote);
        localRef.current = local;
        await api.sftpDownload(hostId, remote, local);
        const contents = await api.readTextFile(local);
        if (cancelled) return;
        setText(contents);
        setOriginal(contents);
        setStatus("");
      } catch (e) {
        if (!cancelled) setError(String(e));
      } finally {
        if (!cancelled) setBusy(false);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [hostId, remote]);

  const dirty = text !== original && !busy && !error;

  async function save() {
    const local = localRef.current;
    if (!local) return;
    setBusy(true);
    setStatus("Saving…");
    try {
      await api.writeTextFile(local, text);
      // Upload to the file's own directory under its own name: the SFTP
      // upload takes a destination path, so the name cannot drift.
      await api.sftpUploadTo(hostId, local, remote);
      setOriginal(text);
      setStatus("Saved to the server");
      onSaved();
    } catch (e) {
      setError(String(e));
      setStatus("");
    } finally {
      setBusy(false);
    }
  }

  function close() {
    if (dirty && !window.confirm(`“${name}” has unsaved changes. Close anyway?`)) return;
    if (localRef.current) api.discardScratch(localRef.current).catch(() => {});
    onClose();
  }

  return (
    <div className="modal-backdrop" onClick={close}>
      <div className="modal editor" onClick={(e) => e.stopPropagation()}>
        <h2>
          {name}
          {dirty && <span className="dirty-dot" title="Unsaved changes">●</span>}
        </h2>
        <div className="editor-path">{remote}</div>

        {error ? (
          <div className="error">{error}</div>
        ) : (
          <textarea
            className="editor-area"
            spellCheck={false}
            value={text}
            disabled={busy && !text}
            onChange={(e) => setText(e.target.value)}
            onKeyDown={(e) => {
              // ⌘S / Ctrl-S saves, which is what hands reach for.
              if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "s") {
                e.preventDefault();
                if (dirty) save();
              }
            }}
          />
        )}

        <div className="modal-actions">
          <span className="editor-status">{status}</span>
          <button onClick={close} disabled={busy && !error}>
            Close
          </button>
          {!error && (
            <button className="primary" onClick={save} disabled={!dirty || busy}>
              {busy ? "Saving…" : "Save to server"}
            </button>
          )}
        </div>
      </div>
    </div>
  );
}
