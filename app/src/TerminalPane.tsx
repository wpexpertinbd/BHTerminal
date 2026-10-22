import { useEffect, useRef, useState } from "react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { api, Channel, decodeChunk, type PtyEvent } from "./api";
import { TERMINAL_THEMES, type TerminalFont } from "./theme";
import { ContextMenu, type MenuAction } from "./ContextMenu";
import { readText, writeText } from "@tauri-apps/plugin-clipboard-manager";
import { findHostKeyChange, type HostKeyChange } from "./hostKey";
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
  /** Family and size for the terminal text. */
  font: TerminalFont;
  onReady: (paneId: string) => void;
  /** Where a keystroke goes. The pane does NOT write to its own session
   *  directly: with broadcast on, what is typed here belongs to every pane in
   *  the tab, and only the tab knows that. */
  onInput: (paneKey: string, data: string) => void;
  /** `hostKey` is set when ssh refused because the server's key changed. */
  onExit: (paneKey: string, code: number | null, hostKey: HostKeyChange | null) => void;
  onTitle: (paneKey: string, title: string) => void;
}

export function TerminalPane({
  paneKey,
  hostId,
  visible,
  focused,
  mode,
  font,
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
  const fontRef = useRef(font);
  fontRef.current = font;
  const [menu, setMenu] = useState<{ x: number; y: number; items: MenuAction[] } | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

  // Created exactly once per pane. ⚠️ Never tear this down on tab switch:
  // the session lives in Rust, and disposing the terminal here would throw
  // away the scrollback the user is switching back to.
  useEffect(() => {
    const term = new Terminal({
      fontFamily: fontRef.current.family,
      fontSize: fontRef.current.size,
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

    // The start of a session, kept as text: ssh's host-key refusal comes
    // before any login, so the first few KB are all that need reading.
    const decoder = new TextDecoder();
    let opening = "";
    const channel = new Channel<PtyEvent>();
    channel.onmessage = (msg) => {
      if (msg.type === "output") {
        const bytes = decodeChunk(msg.data);
        if (opening.length < 16384) opening += decoder.decode(bytes, { stream: true });
        term.write(bytes);
      } else {
        onExit(paneKey, msg.code, hostId ? findHostKeyChange(opening, msg.code) : null);
      }
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

  // A font change resizes the CELLS, not the element, so the ResizeObserver
  // never fires — the grid has to be re-fitted by hand or the remote keeps
  // the old columns and wraps in the wrong place.
  useEffect(() => {
    const term = termRef.current;
    const fit = fitRef.current;
    if (!term || !fit) return;
    term.options.fontFamily = font.family;
    term.options.fontSize = font.size;
    if (!hostRef.current?.offsetParent) return; // hidden: cannot measure
    requestAnimationFrame(() => {
      try { fit.fit(); } catch { return; }
      if (paneIdRef.current) api.resize(paneIdRef.current, term.cols, term.rows);
    });
  }, [font.family, font.size]);

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

  /** Everything in the scrollback, trailing blank lines dropped — what
   *  "copy all" has to mean in a terminal, since the visible screen is
   *  usually the least interesting part of a long build log. */
  function allText(): string {
    const term = termRef.current;
    if (!term) return "";
    const buffer = term.buffer.active;
    const lines: string[] = [];
    for (let i = 0; i < buffer.length; i++) {
      lines.push(buffer.getLine(i)?.translateToString(true) ?? "");
    }
    while (lines.length > 0 && lines[lines.length - 1].trim() === "") lines.pop();
    return lines.join("\n");
  }

  async function copy(text: string, what: string) {
    if (!text) {
      setNotice("Nothing to copy");
      return;
    }
    try {
      // The plugin goes through the OS clipboard directly. navigator.clipboard
      // needs a secure context and a user gesture, and a webview can refuse it
      // with no explanation — which is not a thing to discover mid-paste.
      await writeText(text);
      setNotice(`${what} copied`);
    } catch {
      try {
        await navigator.clipboard.writeText(text);
        setNotice(`${what} copied`);
      } catch {
        setNotice("The clipboard refused the copy");
      }
    }
  }

  function openMenu(event: React.MouseEvent) {
    event.preventDefault();
    event.stopPropagation();
    const term = termRef.current;
    const selection = term?.getSelection() ?? "";
    setMenu({
      x: event.clientX,
      y: event.clientY,
      items: [
        {
          label: "Copy",
          disabled: !selection,
          run: () => copy(selection, "Selection"),
        },
        { label: "Copy everything", run: () => copy(allText(), "Scrollback") },
        {
          label: "Paste",
          run: async () => {
            try {
              const text = await readText();
              // Through the same path as typing, so broadcast applies and a
              // split tab pastes into every pane when it is on.
              if (text) onInputRef.current(paneKey, text);
              else setNotice("The clipboard is empty");
            } catch {
              setNotice("Could not read the clipboard — ⌘V / Ctrl-V still works");
            }
          },
        },
        { divider: true },
        { label: "Select all", run: () => term?.selectAll() },
        {
          label: "Clear selection",
          disabled: !selection,
          run: () => term?.clearSelection(),
        },
        { divider: true },
        {
          label: "Clear scrollback",
          run: () => {
            term?.clear();
            setNotice("Scrollback cleared");
          },
        },
      ],
    });
  }

  // A one-line confirmation that fades: a copy with no feedback leaves you
  // wondering whether it took.
  useEffect(() => {
    if (!notice) return;
    const timer = setTimeout(() => setNotice(null), 1800);
    return () => clearTimeout(timer);
  }, [notice]);

  // ⚠️ The element xterm is opened into holds xterm's OWN DOM, so nothing of
  // React's goes inside it — the notice and the menu are siblings in a
  // wrapper instead.
  return (
    <div
      className="term-wrap"
      data-pane-key={paneKey}
      style={{ display: visible ? "block" : "none" }}
      onContextMenu={openMenu}
    >
      <div className="term-host" ref={hostRef} onMouseDown={() => termRef.current?.focus()} />
      {notice && <div className="term-notice">{notice}</div>}
      {menu && (
        <ContextMenu x={menu.x} y={menu.y} items={menu.items} onClose={() => setMenu(null)} />
      )}
    </div>
  );
}
