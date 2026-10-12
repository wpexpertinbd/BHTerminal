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
