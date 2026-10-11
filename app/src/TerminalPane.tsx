import { useEffect, useRef } from "react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { api, Channel, decodeChunk, type PtyEvent } from "./api";
import "@xterm/xterm/css/xterm.css";

interface Props {
  /** Stable key for this pane; the session is created once per pane. */
  paneKey: string;
  hostId: string | null; // null = local shell
  active: boolean;
  onReady: (paneId: string) => void;
  onExit: (paneKey: string, code: number | null) => void;
  onTitle: (paneKey: string, title: string) => void;
}

export function TerminalPane({ paneKey, hostId, active, onReady, onExit, onTitle }: Props) {
  const hostRef = useRef<HTMLDivElement | null>(null);
  const termRef = useRef<Terminal | null>(null);
  const fitRef = useRef<FitAddon | null>(null);
  const paneIdRef = useRef<string | null>(null);

  // Created exactly once per pane. ⚠️ Never tear this down on tab switch:
  // the session lives in Rust, and disposing the terminal here would throw
  // away the scrollback the user is switching back to.
  useEffect(() => {
    const term = new Terminal({
      fontFamily: 'Menlo, "Cascadia Mono", "DejaVu Sans Mono", monospace',
      fontSize: 13,
      cursorBlink: true,
      allowProposedApi: true,
      theme: { background: "#1e1e1e", foreground: "#e6e6e6" },
    });
    const fit = new FitAddon();
    term.loadAddon(fit);
    term.open(hostRef.current!);
    try { fit.fit(); } catch { /* not laid out yet */ }
    termRef.current = term;
    fitRef.current = fit;

    // The window title is where the remote working directory comes from —
    // distro shells publish it on every prompt, so cwd-follow needs no hook
    // injected into the user's session.
    term.onTitleChange((t) => onTitle(paneKey, t));

    const channel = new Channel<PtyEvent>();
    channel.onmessage = (msg) => {
      if (msg.type === "output") term.write(decodeChunk(msg.data));
      else onExit(paneKey, msg.code);
    };

    const cols = term.cols || 80;
    const rows = term.rows || 24;
    const start = hostId
      ? api.openTerminal(hostId, cols, rows, channel)
      : api.openLocalShell(cols, rows, channel);

    start
      .then((paneId) => {
        paneIdRef.current = paneId;
        onReady(paneId);
        term.onData((data) => api.write(paneId, data));
        term.focus();
      })
      .catch((e) => term.write(`\r\n\x1b[31m${String(e)}\x1b[0m\r\n`));

    const onResize = () => {
      if (!active) return;
      try { fit.fit(); } catch { return; }
      if (paneIdRef.current) api.resize(paneIdRef.current, term.cols, term.rows);
    };
    window.addEventListener("resize", onResize);

    return () => {
      window.removeEventListener("resize", onResize);
      term.dispose();
      // The ssh process is NOT killed here — closing the tab does that
      // explicitly. Unmounting happens for reasons the user didn't ask for.
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Becoming visible: xterm can't measure a hidden element, so re-fit and
  // tell the remote the new size.
  useEffect(() => {
    if (!active) return;
    const term = termRef.current;
    const fit = fitRef.current;
    if (!term || !fit) return;
    requestAnimationFrame(() => {
      try { fit.fit(); } catch { return; }
      if (paneIdRef.current) api.resize(paneIdRef.current, term.cols, term.rows);
      term.focus();
    });
  }, [active]);

  return <div className="term-host" style={{ display: active ? "block" : "none" }} ref={hostRef} />;
}
