/** Light or dark, following the system unless the user says otherwise.
 *
 *  The system case is handled in CSS by `prefers-color-scheme`, which the
 *  webview reports from the OS on all three platforms — so "System" needs no
 *  code at all beyond removing the override. An explicit choice is written to
 *  `data-theme` on <html>, which the stylesheet gives priority over the media
 *  query. */
export type ThemeChoice = "system" | "light" | "dark";

const KEY = "bhterminal.theme";

export function loadTheme(): ThemeChoice {
  // A per-viewer convenience, so browser storage is the right place — but it
  // can throw or come back empty, and the app has to render either way.
  try {
    const stored = localStorage.getItem(KEY);
    if (stored === "light" || stored === "dark" || stored === "system") return stored;
  } catch {
    /* storage unavailable: fall back to following the system */
  }
  return "system";
}

export function applyTheme(choice: ThemeChoice) {
  const root = document.documentElement;
  if (choice === "system") {
    root.removeAttribute("data-theme");
  } else {
    root.setAttribute("data-theme", choice);
  }
  try {
    localStorage.setItem(KEY, choice);
  } catch {
    /* the choice still applies to this run */
  }
}

/** Applied before React renders, so the first paint is already the right
 *  colour instead of flashing light and then correcting itself. */
export function initTheme(): ThemeChoice {
  const choice = loadTheme();
  applyTheme(choice);
  return choice;
}

/** What the choice actually resolves to right now — "system" has to be asked
 *  of the OS, and the answer can change while the app is open. */
export function resolvedMode(choice: ThemeChoice): "light" | "dark" {
  if (choice !== "system") return choice;
  try {
    return window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light";
  } catch {
    return "light";
  }
}

/** Calls back when the SYSTEM appearance changes, so a window left on "Auto"
 *  follows the desktop without being reopened. Returns an unsubscribe. */
export function watchSystemMode(onChange: () => void): () => void {
  try {
    const mq = window.matchMedia("(prefers-color-scheme: dark)");
    mq.addEventListener("change", onChange);
    return () => mq.removeEventListener("change", onChange);
  } catch {
    return () => {};
  }
}

/** The terminal's own colours. A light shell on a light desktop is what was
 *  asked for; the palette is the UI's, not a theme of its own. */
export const TERMINAL_THEMES = {
  light: {
    background: "#ffffff",
    foreground: "#1d1d1f",
    cursor: "#0d6efd",
    cursorAccent: "#ffffff",
    selectionBackground: "#cfe2ff",
  },
  dark: {
    background: "#1e1e1e",
    foreground: "#e6e6e6",
    cursor: "#e6e6e6",
    cursorAccent: "#1e1e1e",
    selectionBackground: "#3a3f4b",
  },
} as const;

/** The terminal's font, kept with the theme because they are one choice in
 *  the user's head: "how the terminal looks".
 *
 *  Each entry is a CSS stack ending in `monospace`, so a font that is not
 *  installed falls back instead of breaking the layout — the preview in the
 *  Appearance sheet is what shows whether it is really there. */
export const TERMINAL_FONTS: { label: string; stack: string }[] = [
  { label: "System default", stack: 'Menlo, "Cascadia Mono", "DejaVu Sans Mono", monospace' },
  { label: "SF Mono", stack: '"SF Mono", Menlo, monospace' },
  { label: "Menlo", stack: "Menlo, monospace" },
  { label: "Monaco", stack: "Monaco, monospace" },
  { label: "Consolas", stack: "Consolas, monospace" },
  { label: "Cascadia Mono", stack: '"Cascadia Mono", "Cascadia Code", monospace' },
  { label: "JetBrains Mono", stack: '"JetBrains Mono", monospace' },
  { label: "Fira Code", stack: '"Fira Code", monospace' },
  { label: "DejaVu Sans Mono", stack: '"DejaVu Sans Mono", monospace' },
  { label: "Ubuntu Mono", stack: '"Ubuntu Mono", monospace' },
  { label: "Courier New", stack: '"Courier New", monospace' },
];

export interface TerminalFont {
  family: string;
  size: number;
}

export const FONT_LIMITS = { min: 9, max: 24 } as const;
const DEFAULT_FONT: TerminalFont = { family: TERMINAL_FONTS[0].stack, size: 13 };
const FONT_KEY = "bhterminal.terminalFont";

export function loadFont(): TerminalFont {
  try {
    const raw = localStorage.getItem(FONT_KEY);
    if (!raw) return { ...DEFAULT_FONT };
    const saved = JSON.parse(raw) as Partial<TerminalFont>;
    const size = Number(saved.size);
    return {
      family: typeof saved.family === "string" && saved.family ? saved.family : DEFAULT_FONT.family,
      size: Number.isFinite(size)
        ? Math.min(FONT_LIMITS.max, Math.max(FONT_LIMITS.min, Math.round(size)))
        : DEFAULT_FONT.size,
    };
  } catch {
    return { ...DEFAULT_FONT };
  }
}

export function saveFont(font: TerminalFont) {
  try {
    localStorage.setItem(FONT_KEY, JSON.stringify(font));
  } catch {
    /* the choice still applies to this run */
  }
}

export const DEFAULT_TERMINAL_FONT = DEFAULT_FONT;
