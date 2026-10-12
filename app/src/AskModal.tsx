import { useEffect, useRef, useState } from "react";

/** Asking for a name, or for a yes.
 *
 *  ⚠️ `window.prompt` does NOT exist in a Tauri webview — it returns null
 *  without showing anything, so every "Rename…" and "New folder…" built on
 *  it silently did nothing. `confirm` is in the same family and not worth
 *  betting a delete on either, so both are drawn here instead. */
interface Props {
  title: string;
  /** Absent for a confirmation; present (even empty) for a text question. */
  value?: string;
  detail?: string;
  confirmLabel?: string;
  danger?: boolean;
  onCancel: () => void;
  onSubmit: (value: string) => void;
}

export function AskModal({ title, value, detail, confirmLabel, danger, onCancel, onSubmit }: Props) {
  const [text, setText] = useState(value ?? "");
  const input = useRef<HTMLInputElement | null>(null);

  useEffect(() => {
    // Select the stem of a filename, the way a rename field should: the
    // extension is rarely the part being changed.
    const el = input.current;
    if (!el) return;
    el.focus();
    const dot = el.value.lastIndexOf(".");
    el.setSelectionRange(0, dot > 0 ? dot : el.value.length);
  }, []);

  const isText = value !== undefined;
  const submit = () => {
    if (isText && !text.trim()) return;
    onSubmit(text.trim());
  };

  return (
    <div className="modal-backdrop" onClick={onCancel}>
      <div className="modal ask" onClick={(e) => e.stopPropagation()}>
        <h2>{title}</h2>
        {detail && <p className="done-line">{detail}</p>}
        {isText && (
          <input
            ref={input}
            value={text}
            onChange={(e) => setText(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") submit();
              if (e.key === "Escape") onCancel();
            }}
          />
        )}
        <div className="modal-actions">
          <button onClick={onCancel}>Cancel</button>
          <button
            className={danger ? "danger-btn" : "primary"}
            onClick={submit}
            disabled={isText && !text.trim()}
          >
            {confirmLabel ?? "OK"}
          </button>
        </div>
      </div>
    </div>
  );
}
