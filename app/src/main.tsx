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

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <Root />
  </React.StrictMode>
);
