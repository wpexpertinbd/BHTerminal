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
