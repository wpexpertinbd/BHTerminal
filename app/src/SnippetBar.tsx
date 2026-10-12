import { useState } from "react";
import { api, newId, type Snippet } from "./api";
import { AskModal } from "./AskModal";

interface Props {
  snippets: Snippet[];
  /** Null when no terminal is in front: snippets are sent to a shell, so
   *  there has to be one. */
  onSend: ((command: string) => void) | null;
  onChanged: (snippets: Snippet[]) => void;
}

export function SnippetBar({ snippets, onSend, onChanged }: Props) {
  const [open, setOpen] = useState(false);
  const [editing, setEditing] = useState<Snippet | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [confirming, setConfirming] = useState<Snippet | null>(null);

  async function save(s: Snippet) {
    if (!s.name.trim() || !s.command.trim()) {
      return setError("A snippet needs a name and a command.");
    }
    try {
      const saved = await api.saveSnippet({ ...s, name: s.name.trim(), command: s.command });
      onChanged(
        snippets.some((x) => x.id === saved.id)
          ? snippets.map((x) => (x.id === saved.id ? saved : x))
          : [...snippets, saved]
      );
      setEditing(null);
      setError(null);
    } catch (e) {
      setError(String(e));
    }
  }

  async function remove(s: Snippet) {
    await api.deleteSnippet(s.id);
    onChanged(snippets.filter((x) => x.id !== s.id));
  }

  return (
    <div className="snippets">
      <div className="row folder" onClick={() => setOpen((o) => !o)}>
        <span className="twisty">{open ? "▾" : "▸"}</span>
        <span>Snippets</span>
        <span className="count">{snippets.length}</span>
        <span className="row-actions">
          <button
            title="New snippet"
            onClick={(e) => {
              e.stopPropagation();
              setOpen(true);
              setEditing({ id: newId(), name: "", command: "", sortOrder: 0 });
            }}
          >
            ＋
          </button>
        </span>
      </div>

      {open && (
        <div className="children">
          {snippets.length === 0 && !editing && <div className="empty">No snippets yet.</div>}
          {snippets.map((s) => (
            <div
              key={s.id}
              className="row host"
              title={onSend ? s.command : "Open a terminal tab to run a snippet"}
              onClick={() => onSend?.(s.command)}
            >
              <span className="host-text">
                <span className="host-name">{s.name}</span>
                <span className="host-sub">{s.command}</span>
              </span>
              <span className="row-actions">
                <button
                  title="Edit"
                  onClick={(e) => {
                    e.stopPropagation();
                    setEditing(s);
                  }}
                >
                  ✎
                </button>
                <button
                  title="Delete"
                  onClick={(e) => {
                    e.stopPropagation();
                    setConfirming(s);
                  }}
                >
                  🗑
                </button>
              </span>
            </div>
          ))}
        </div>
      )}

      {confirming && (
        <AskModal
          title={`Delete the snippet “${confirming.name}”?`}
          confirmLabel="Delete"
          danger
          onCancel={() => setConfirming(null)}
          onSubmit={() => {
            const s = confirming;
            setConfirming(null);
            remove(s);
          }}
        />
      )}

      {editing && (
        <div className="modal-backdrop" onClick={() => setEditing(null)}>
          <div className="modal" onClick={(e) => e.stopPropagation()}>
            <h2>{snippets.some((s) => s.id === editing.id) ? "Edit snippet" : "New snippet"}</h2>
            <label className="field">
              <span>Name</span>
              <input
                autoFocus
                value={editing.name}
                onChange={(e) => setEditing({ ...editing, name: e.target.value })}
              />
            </label>
            <label className="field">
              <span>Command</span>
              <textarea
                rows={3}
                value={editing.command}
                onChange={(e) => setEditing({ ...editing, command: e.target.value })}
              />
              <small>
                Sent to the terminal that is in front, exactly as typed — no newline is added, so
                nothing runs until you press Return.
              </small>
            </label>
            {error && <div className="error">{error}</div>}
            <div className="modal-actions">
              <button onClick={() => setEditing(null)}>Cancel</button>
              <button className="primary" onClick={() => save(editing)}>
                Save
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
