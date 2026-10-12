import { useEffect, useRef } from "react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { api, Channel, decodeChunk, type PtyEvent } from "./api";
import { TERMINAL_THEMES } from "./theme";
import "@xterm/xterm/css/xterm.css";

interface Props {
  /** Stable key for this pane; the session is created once per pane. */
  paneKey: string;
  hostId: string | null; // null = local shell
  /** Its tab is the one in front. A hidden terminal cannot be measured, so
   *  this is what re-fits it. */
  visible: boolean;
  /** It is the pane within that tab that has focus. With a split tab several
   *  panes are visible and only one is focused — which is why these are two
   *  props and not one. */
  focused: boolean;
  /** Light or dark, already resolved — the terminal follows the rest of the
   *  window rather than being dark whatever the desktop is doing. */
  mode: "light" | "dark";
  onReady: (paneId: string) => void;
  /** Where a keystroke goes. The pane does NOT write to its own session
   *  directly: with broadcast on, what is typed here belongs to every pane in
   *  the tab, and only the tab knows that. */
  onInput: (paneKey: string, data: string) => void;
  onExit: (paneKey: string, code: number | null) => void;
  onTitle: (paneKey: string, title: string) => void;
}

export function TerminalPane({
  paneKey,
  hostId,
  visible,
  focused,
  mode,
  onReady,
  onInput,
  onExit,
  onTitle,
}: Props) {
  const hostRef = useRef<HTMLDivElement | null>(null);
  const termRef = useRef<Terminal | null>(null);
  const fitRef = useRef<FitAddon | null>(null);
  const paneIdRef = useRef<string | null>(null);
  // xterm's handler is registered once, so it must not close over the first
  // render's callback — broadcast can be switched on after the pane exists.
  const onInputRef = useRef(onInput);
  onInputRef.current = onInput;
  // Read at creation time; the effect below keeps it current afterwards.
  const modeRef = useRef(mode);
  modeRef.current = mode;

  // Created exactly once per pane. ⚠️ Never tear this down on tab switch:
  // the session lives in Rust, and disposing the terminal here would throw
  // away the scrollback the user is switching back to.
  useEffect(() => {
    const term = new Terminal({
      fontFamily: 'Menlo, "Cascadia Mono", "DejaVu Sans Mono", monospace',
      fontSize: 13,
      cursorBlink: true,
      allowProposedApi: true,
      theme: { ...TERMINAL_THEMES[modeRef.current] },
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
        term.onData((data) => onInputRef.current(paneKey, data));
        term.focus();
      })
      .catch((e) => term.write(`\r\n\x1b[31m${String(e)}\x1b[0m\r\n`));

    // A ResizeObserver rather than a window listener: splitting a tab resizes
    // its siblings with no window resize at all, and a terminal whose columns
    // are wrong wraps every line in the wrong place.
    let queued = false;
    const observer = new ResizeObserver(() => {
      if (queued) return;
      queued = true;
      requestAnimationFrame(() => {
        queued = false;
        if (!hostRef.current?.offsetParent) return; // hidden: cannot measure
        try { fit.fit(); } catch { return; }
        if (paneIdRef.current) api.resize(paneIdRef.current, term.cols, term.rows);
      });
    });
    observer.observe(hostRef.current!);

    return () => {
      observer.disconnect();
      term.dispose();
      // The ssh process is NOT killed here — closing the tab does that
      // explicitly. Unmounting happens for reasons the user didn't ask for.
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // The appearance changed under a live terminal: repaint it without
  // disturbing the session or its scrollback.
  useEffect(() => {
    const term = termRef.current;
    if (term) term.options.theme = { ...TERMINAL_THEMES[mode] };
  }, [mode]);

  // Becoming visible: xterm can't measure a hidden element, so re-fit and
  // tell the remote the new size.
  useEffect(() => {
    if (!visible) return;
    const term = termRef.current;
    const fit = fitRef.current;
    if (!term || !fit) return;
    requestAnimationFrame(() => {
      try { fit.fit(); } catch { return; }
      if (paneIdRef.current) api.resize(paneIdRef.current, term.cols, term.rows);
      if (focused) term.focus();
    });
  }, [visible, focused]);

  return (
    <div
      className="term-host"
      style={{ display: visible ? "block" : "none" }}
      ref={hostRef}
      onMouseDown={() => termRef.current?.focus()}
    />
  );
}
