import React from "react";
import ReactDOM from "react-dom/client";
import Root from "./Root";
import "./styles.css";
import { initTheme } from "./theme";

// Before the first render: otherwise the window paints light and corrects
// itself a frame later, which reads as a flash on a dark desktop.
initTheme();

// The webview's own context menu belongs to a browser, not to this app:
// right-clicking a server's file listing should not offer "Search with
// Google" or "Translate". Text fields keep theirs, where cut/copy/paste and
// spelling are genuinely useful.
document.addEventListener(
  "contextmenu",
  (event) => {
    const target = event.target as HTMLElement | null;
    const editable =
      target?.closest("input, textarea, [contenteditable='true']") !== null ||
      // A selection the user made deliberately is worth keeping the menu for.
      (window.getSelection()?.toString().length ?? 0) > 0;
    if (!editable) event.preventDefault();
  },
  { capture: true }
);

// ⚠️ macOS can capitalise the first letter of an empty text field ("Capitalize
// words automatically" in Keyboard settings reaches web views): a username
// typed as `root` was saved as `Root`, and Linux logins are case-sensitive, so
// that host could never log in. Every box in this app holds something exact —
// a login, an address, a command, a file name — so none of them may be
// "helped". The attributes are set the moment a field exists, before anyone can
// focus it, and again on focus for anything missed.
function noAutoFix(el: Element) {
  if (el instanceof HTMLInputElement || el instanceof HTMLTextAreaElement) {
    el.setAttribute("autocapitalize", "off");
    el.setAttribute("autocorrect", "off");
    el.setAttribute("spellcheck", "false");
  }
}
function noAutoFixTree(root: ParentNode) {
  root.querySelectorAll("input, textarea").forEach(noAutoFix);
}
noAutoFixTree(document);
new MutationObserver((records) => {
  for (const r of records) {
    r.addedNodes.forEach((n) => {
      if (n instanceof Element) {
        noAutoFix(n);
        noAutoFixTree(n);
      }
    });
  }
}).observe(document.documentElement, { childList: true, subtree: true });
document.addEventListener("focusin", (e) => e.target instanceof Element && noAutoFix(e.target), true);

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <Root />
  </React.StrictMode>
);
