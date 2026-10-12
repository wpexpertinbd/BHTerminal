import { useState } from "react";
import {
  applyTheme,
  DEFAULT_TERMINAL_FONT,
  FONT_LIMITS,
  TERMINAL_FONTS,
  type TerminalFont,
  type ThemeChoice,
} from "./theme";

interface Props {
  theme: ThemeChoice;
  font: TerminalFont;
  onTheme: (choice: ThemeChoice) => void;
  onFont: (font: TerminalFont) => void;
  onClose: () => void;
}

/** Theme and terminal font in one place, because they are one question:
 *  how the terminal looks. Everything applies as it is changed — the
 *  terminals behind this sheet repaint live — so the only button is Done. */
export function AppearanceSheet({ theme, font, onTheme, onFont, onClose }: Props) {
  const [local, setLocal] = useState(font);

  function setFont(next: TerminalFont) {
    const size = Math.min(FONT_LIMITS.max, Math.max(FONT_LIMITS.min, Math.round(next.size)));
    const clamped = { ...next, size };
    setLocal(clamped);
    onFont(clamped);
  }

  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div className="modal appearance" onClick={(e) => e.stopPropagation()}>
        <h2>Appearance</h2>

        <label className="field">
          <span>Theme</span>
          <div className="seg">
            {(["system", "light", "dark"] as ThemeChoice[]).map((t) => (
              <button
                key={t}
                className={theme === t ? "on" : ""}
                onClick={() => {
                  applyTheme(t);
                  onTheme(t);
                }}
              >
                {t === "system" ? "Auto" : t === "light" ? "Light" : "Dark"}
              </button>
            ))}
          </div>
          <small>Auto follows your desktop while the app is open.</small>
        </label>

        <div className="row2">
          <label className="field">
            <span>Terminal font</span>
            <select
              value={local.family}
              onChange={(e) => setFont({ ...local, family: e.target.value })}
            >
              {TERMINAL_FONTS.map((f) => (
                <option key={f.label} value={f.stack}>
                  {f.label}
                </option>
              ))}
            </select>
          </label>
          <label className="field narrow">
            <span>Size</span>
            <input
              type="number"
              min={FONT_LIMITS.min}
              max={FONT_LIMITS.max}
              value={local.size}
              onChange={(e) => setFont({ ...local, size: Number(e.target.value) || local.size })}
            />
          </label>
        </div>

        {/* The preview is the honest check: a font that is not installed
            falls back silently, and this is where that shows. */}
        <div className="field">
          <span>Preview</span>
          <pre
            className="font-preview"
            style={{ fontFamily: local.family, fontSize: `${local.size}px` }}
          >
            {"root@server:~# systemctl status nginx\nActive: active (running) 0O1lI|-_ {}[]()<>"}
          </pre>
        </div>

        <div className="modal-actions">
          <button
            onClick={() => {
              setFont({ ...DEFAULT_TERMINAL_FONT });
            }}
          >
            Reset font
          </button>
          <button className="primary" onClick={onClose}>
            Done
          </button>
        </div>
      </div>
    </div>
  );
}
