import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import "./styles.css";
import { initTheme } from "./theme";

// Before the first render: otherwise the window paints light and corrects
// itself a frame later, which reads as a flash on a dark desktop.
initTheme();

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>
);
