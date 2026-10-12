import { useEffect, useMemo, useState } from "react";
import { save as saveFile } from "@tauri-apps/plugin-dialog";
import { api, blankHost, newId, type Host, type HostFolder, type Workspace } from "./api";
import { AskModal } from "./AskModal";
import { HostEditor } from "./HostEditor";
import { TransferSheet } from "./TransferSheet";
import { useUpdateCheck } from "./useUpdateCheck";
import { useResizablePanes } from "./useResizablePanes";
import { loadFont, loadTheme, resolvedMode, saveFont, watchSystemMode, type TerminalFont, type ThemeChoice } from "./theme";
import { AppearanceSheet } from "./AppearanceSheet";
import { SnippetBar } from "./SnippetBar";
import { TunnelBar } from "./TunnelBar";
import { TerminalPane } from "./TerminalPane";
import { FilePane } from "./FilePane";
import { VncPane } from "./VncPane";

interface Pane {
  key: string;
  kind: "terminal" | "vnc";
  hostId: string | null;
  title: string;
  /** Set when ssh exits unexpectedly: the tab stays so it can be reconnected,
   *  rather than vanishing and taking the error message with it. */
  exited: boolean;
}

/** A tab holds one or more panes. Splitting a tab adds a pane to it rather
 *  than nesting splits inside splits: a row or a column of terminals covers
 *  what people actually do with a split, and a tree would be a lot of
 *  machinery for the rest. */
interface Tab {
  key: string;
  panes: Pane[];
  axis: "row" | "column";
  /** Which of its panes has focus — snippets, the file browser and the
   *  forwards bar all follow this one. */
  focused: string;
  /** Typing reaches every pane in THIS tab. Scoped to the tab on purpose:
   *  a global flag would quietly reach the sessions in other tabs, which is
   *  the one mistake this feature cannot afford. */
  broadcast: boolean;
}

let paneSeq = 0;
let tabSeq = 0;

function newPane(kind: Pane["kind"], hostId: string | null, title: string): Pane {
  return { key: `pane-${++paneSeq}`, kind, hostId, title, exited: false };
}

function newTab(pane: Pane): Tab {
  return { key: `tab-${++tabSeq}`, panes: [pane], axis: "row", focused: pane.key, broadcast: false };
}

/** Mirrors bhterm_core::title::parse_title — only something unambiguously a
 *  path is accepted, so ordinary titles ("vim foo", "htop") are ignored, and
 *  a path containing a colon is skipped rather than mis-split. */
function parseTitlePath(title: string): string | null {
  const t = title.trim();
  if (!t) return null;
  const i = t.lastIndexOf(":");
  const candidate = (i >= 0 ? t.slice(i + 1) : t).trim();
  const looksLikePath =
    candidate === "~" || candidate.startsWith("~/") || candidate.startsWith("/");
  if (!looksLikePath) return null;
  // eslint-disable-next-line no-control-regex
  if (/[\x00-\x1f\x7f]/.test(candidate)) return null;
  return candidate;
}

export default function App() {
  const [workspace, setWorkspace] = useState<Workspace | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [tabs, setTabs] = useState<Tab[]>([]);
  const [activeTabKey, setActiveTabKey] = useState<string | null>(null);
  const [selected, setSelected] = useState<string | null>(null);
  const [expanded, setExpanded] = useState<Set<string>>(new Set());
  /** Latest cwd each host's terminal reported through its window title. */
  const [cwdByHost, setCwdByHost] = useState<Record<string, string>>({});
  const [followTerminal, setFollowTerminal] = useState(false);
  /** The host being edited, and whether it is new (so the editor knows not to
   *  look for a stored secret that cannot exist yet). */
  const [editing, setEditing] = useState<{ host: Host; isNew: boolean } | null>(null);
  const [menuOpen, setMenuOpen] = useState(false);
  const [transfer, setTransfer] = useState<"export" | "restore" | "import" | null>(null);
  /** See AskModal: window.prompt does not exist in this webview. */
  const [ask, setAsk] = useState<
    | null
    | { title: string; value?: string; detail?: string; confirmLabel?: string; danger?: boolean;
        onSubmit: (value: string) => void }
  >(null);
  const [version, setVersion] = useState<string>("");
  const [theme, setTheme] = useState<ThemeChoice>(loadTheme);
  /** What that choice means right now. "Auto" has to be re-asked whenever the
   *  desktop switches, which is what the watcher is for. */
  const [mode, setMode] = useState<"light" | "dark">(() => resolvedMode(loadTheme()));
  const [font, setFont] = useState<TerminalFont>(loadFont);
  const [appearanceOpen, setAppearanceOpen] = useState(false);
  /** Where the focused pane is recording, if it is. Read from Rust rather
   *  than tracked here, so it stays right across tab switches. */
  const [logPath, setLogPath] = useState<string | null>(null);
  const update = useUpdateCheck(setVersion);
  const { widths, collapsed, startDrag, reset, toggleCollapsed } = useResizablePanes();

  useEffect(() => {
    api.loadWorkspace().then(setWorkspace).catch((e) => setError(String(e)));
  }, []);

  // Follow the desktop while the choice is "Auto".
  useEffect(() => {
    api.setWindowTheme(theme === "system" ? null : theme).catch(() => {});
    if (theme !== "system") return;
    return watchSystemMode(() => setMode(resolvedMode("system")));
  }, [theme]);

  const hostsByFolder = useMemo(() => {
    const map = new Map<string | null, Host[]>();
    for (const h of workspace?.store.hosts ?? []) {
      const k = h.folderID ?? null;
      if (!map.has(k)) map.set(k, []);
      map.get(k)!.push(h);
    }
    for (const list of map.values()) list.sort((a, b) => a.sortOrder - b.sortOrder);
    return map;
  }, [workspace]);

  const rootFolders = useMemo(
    () =>
      (workspace?.store.folders ?? [])
        .filter((f) => !f.parentFolderID)
        .sort((a, b) => a.sortOrder - b.sortOrder),
    [workspace]
  );

  function connect(host: Host) {
    // Reuse an open pane for the same host rather than stacking duplicates —
    // double-clicking a host twice is almost always "show me that session".
    for (const tab of tabs) {
      const hit = tab.panes.find((p) => p.hostId === host.id && !p.exited);
      if (hit) {
        setActiveTabKey(tab.key);
        setTabs((prev) => prev.map((t) => (t.key === tab.key ? { ...t, focused: hit.key } : t)));
        return;
      }
    }
    openInNewTab(
      newPane(host.connectionType === "vnc" ? "vnc" : "terminal", host.id, host.name)
    );
  }

  function openLocalShell() {
    openInNewTab(newPane("terminal", null, "Local shell"));
  }

  function openInNewTab(pane: Pane) {
    const tab = newTab(pane);
    setTabs((prev) => [...prev, tab]);
    setActiveTabKey(tab.key);
  }

  /** Splits the focused pane: a second session to the same host, beside it or
   *  under it. A VNC pane is not split — two copies of one desktop in half
   *  the space is not useful, and each costs a full connection. */
  function splitFocused(axis: Tab["axis"]) {
    const tab = tabs.find((t) => t.key === activeTabKey);
    const source = tab?.panes.find((p) => p.key === tab.focused);
    if (!tab || !source || source.kind !== "terminal") return;
    const pane = newPane("terminal", source.hostId, source.title);
    setTabs((prev) =>
      prev.map((t) =>
        t.key === tab.key ? { ...t, axis, panes: [...t.panes, pane], focused: pane.key } : t
      )
    );
  }

  /** A keystroke, or a snippet. With broadcast on it goes to every pane in
   *  the tab the pane belongs to — and to nothing outside it. */
  function sendInput(paneKey: string, data: string) {
    const tab = tabs.find((t) => t.panes.some((p) => p.key === paneKey));
    // An exited pane is skipped rather than written to: its session is gone,
    // and "no such terminal" on every keypress is noise, not news — the tab
    // already says it disconnected.
    const targets =
      tab && tab.broadcast && tab.panes.length > 1
        ? tab.panes.filter((p) => !p.exited)
        : tab?.panes.filter((p) => p.key === paneKey && !p.exited) ?? [];
    for (const pane of targets) {
      const id = paneIds.get(pane.key);
      if (id) api.write(id, data).catch((e) => setError(String(e)));
    }
  }

  function toggleBroadcast() {
    if (!activeTabKey) return;
    setTabs((prev) =>
      prev.map((t) => (t.key === activeTabKey ? { ...t, broadcast: !t.broadcast } : t))
    );
  }

  function focusPane(tabKey: string, paneKey: string) {
    setActiveTabKey(tabKey);
    setTabs((prev) => prev.map((t) => (t.key === tabKey ? { ...t, focused: paneKey } : t)));
  }

  function patchPane(paneKey: string, patch: (p: Pane) => Pane) {
    setTabs((prev) =>
      prev.map((t) => ({
        ...t,
        panes: t.panes.map((p) => (p.key === paneKey ? patch(p) : p)),
      }))
    );
  }

  /** Replaces the host in place after a save so the tree updates without a
   *  full reload (which would also lose nothing, but flickers). */
  function mergeHost(saved: Host) {
    setWorkspace((w) => {
      if (!w) return w;
      const hosts = w.store.hosts.some((h) => h.id === saved.id)
        ? w.store.hosts.map((h) => (h.id === saved.id ? saved : h))
        : [...w.store.hosts, saved];
      return { ...w, store: { ...w.store, hosts } };
    });
    if (saved.folderID) setExpanded((e) => new Set(e).add(saved.folderID!));
    setSelected(saved.id);
  }

  function newHost(folderID: string | null) {
    setMenuOpen(false);
    setEditing({ host: blankHost(folderID), isNew: true });
  }

  function newFolder() {
    setMenuOpen(false);
    setAsk({
      title: "New folder",
      value: "",
      confirmLabel: "Create",
      onSubmit: async (name) => {
        setAsk(null);
        try {
          const saved = await api.saveFolder({
            id: newId(),
            name,
            parentFolderID: null,
            sortOrder: 0,
          });
          setWorkspace((w) =>
            w ? { ...w, store: { ...w.store, folders: [...w.store.folders, saved] } } : w
          );
        } catch (e) {
          setError(String(e));
        }
      },
    });
  }

  function removeHost(h: Host) {
    setAsk({
      title: `Delete “${h.name}”?`,
      detail: "Its saved password and key passphrase are deleted with it.",
      confirmLabel: "Delete",
      danger: true,
      onSubmit: async () => {
        setAsk(null);
        try {
          await api.deleteHost(h.id);
          setWorkspace((w) =>
            w ? { ...w, store: { ...w.store, hosts: w.store.hosts.filter((x) => x.id !== h.id) } } : w
          );
          if (selected === h.id) setSelected(null);
        } catch (e) {
          setError(String(e));
        }
      },
    });
  }

  function removeFolder(f: HostFolder) {
    const inside = (hostsByFolder.get(f.id) ?? []).length;
    setAsk({
      title: `Delete “${f.name}”?`,
      detail: inside
        ? `The ${inside} host${inside === 1 ? "" : "s"} inside it go too, with their saved passwords.`
        : "The folder is empty.",
      confirmLabel: "Delete",
      danger: true,
      onSubmit: async () => {
        setAsk(null);
        try {
          await api.deleteFolder(f.id);
          // The delete cascades into subfolders on the Rust side, so reload
          // rather than reproducing that walk here.
          setWorkspace(await api.loadWorkspace());
        } catch (e) {
          setError(String(e));
        }
      },
    });
  }

  const paneIds = useMemo(() => new Map<string, string>(), []);

  /** Ends one pane's session. Closing the last pane of a tab closes the tab —
   *  an empty tab is not a thing anyone wants to look at. */
  function closePane(tabKey: string, paneKey: string) {
    const id = paneIds.get(paneKey);
    if (id) api.close(id); // the ONLY thing that ends a session
    paneIds.delete(paneKey);
    setTabs((prev) => {
      const next: Tab[] = [];
      for (const t of prev) {
        if (t.key !== tabKey) {
          next.push(t);
          continue;
        }
        const panes = t.panes.filter((p) => p.key !== paneKey);
        if (panes.length === 0) continue; // the tab goes with its last pane
        next.push({
          ...t,
          panes,
          focused: t.focused === paneKey ? panes[panes.length - 1].key : t.focused,
        });
      }
      setActiveTabKey((cur) =>
        next.some((t) => t.key === cur) ? cur : next[next.length - 1]?.key ?? null
      );
      return next;
    });
  }

  function closeTab(tabKey: string) {
    const tab = tabs.find((t) => t.key === tabKey);
    if (!tab) return;
    for (const pane of tab.panes) {
      const id = paneIds.get(pane.key);
      if (id) api.close(id);
      paneIds.delete(pane.key);
    }
    setTabs((prev) => {
      const next = prev.filter((t) => t.key !== tabKey);
      setActiveTabKey((cur) => (cur === tabKey ? next[next.length - 1]?.key ?? null : cur));
      return next;
    });
  }

  function toggleFolder(id: string) {
    setExpanded((prev) => {
      const next = new Set(prev);
      next.has(id) ? next.delete(id) : next.add(id);
      return next;
    });
  }

  function renderHost(h: Host) {
    const live = allPanes.some((p) => p.hostId === h.id && !p.exited);
    return (
      <div
        key={h.id}
        className={`row host ${selected === h.id ? "selected" : ""}`}
        onClick={() => setSelected(h.id)}
        onDoubleClick={() => connect(h)}
        title={`${h.username}@${h.hostname}:${h.port}`}
      >
        <span className={`dot ${live ? "live" : ""}`} />
        <span className="host-text">
          <span className="host-name">{h.name}</span>
          <span className="host-sub">
            {h.connectionType === "vnc"
              ? `VNC ${h.hostname}:${h.port}`
              : `${h.username}@${h.hostname}:${h.port}`}
          </span>
        </span>
        <span className="row-actions">
          <button
            title="Edit"
            onClick={(e) => {
              e.stopPropagation();
              setEditing({ host: h, isNew: false });
            }}
          >
            ✎
          </button>
          <button
            title="Delete"
            onClick={(e) => {
              e.stopPropagation();
              removeHost(h);
            }}
          >
            🗑
          </button>
        </span>
      </div>
    );
  }

  function renderFolder(f: HostFolder) {
    const open = expanded.has(f.id);
    const children = hostsByFolder.get(f.id) ?? [];
    return (
      <div key={f.id}>
        <div className="row folder" onClick={() => toggleFolder(f.id)}>
          <span className="twisty">{open ? "▾" : "▸"}</span>
          <span>{f.name}</span>
          <span className="count">{children.length}</span>
          <span className="row-actions">
            <button
              title="New host in this folder"
              onClick={(e) => {
                e.stopPropagation();
                newHost(f.id);
              }}
            >
              ＋
            </button>
            <button
              title="Delete folder"
              onClick={(e) => {
                e.stopPropagation();
                removeFolder(f);
              }}
            >
              🗑
            </button>
          </span>
        </div>
        {open && <div className="children">{children.map(renderHost)}</div>}
      </div>
    );
  }

  const activeTab = tabs.find((t) => t.key === activeTabKey) ?? null;
  /** The focused pane of the tab in front: what the file browser, the
   *  forwards bar, the snippets and the logging button all act on. */
  const active = activeTab?.panes.find((p) => p.key === activeTab.focused) ?? null;
  const activeHost =
    workspace && active?.hostId ? workspace.store.hosts.find((h) => h.id === active.hostId) : undefined;
  const allPanes = tabs.flatMap((t) => t.panes);

  // The focused pane changed: ask Rust what IT is recording to.
  useEffect(() => {
    const id = active ? paneIds.get(active.key) : undefined;
    if (!id) {
      setLogPath(null);
      return;
    }
    api.loggingPath(id).then(setLogPath).catch(() => setLogPath(null));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [active?.key]);

  async function toggleLogging() {
    const pane = active;
    const id = pane ? paneIds.get(pane.key) : undefined;
    if (!pane || !id) return;
    try {
      if (logPath) {
        await api.stopLogging(id);
        setLogPath(null);
        return;
      }
      const path = await saveFile({
        title: "Record this session to",
        defaultPath: await api.suggestedLogName(pane.hostId),
      });
      if (!path) return;
      await api.startLogging(id, path);
      setLogPath(path);
    } catch (e) {
      setError(String(e));
    }
  }

  // SFTP rides the terminal's connection, so drop sessions whose host has no
  // tab left open.
  useEffect(() => {
    const keep = Array.from(new Set(allPanes.map((p) => p.hostId).filter(Boolean) as string[]));
    api.sftpPrune(keep).catch(() => {});
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [tabs]);

  return (
    <div
      className={`app ${collapsed.sidebar ? "hide-sidebar" : ""} ${
        collapsed.files ? "hide-files" : ""
      }`}
      style={
        {
          "--sidebar-w": `${widths.sidebar}px`,
          "--files-w": `${widths.files}px`,
        } as React.CSSProperties
      }
    >
      <aside className="sidebar">
        <div className="sidebar-head">
          <strong>Sessions</strong>
          <button
            className="collapse"
            title="Hide the sessions list"
            onClick={() => toggleCollapsed("sidebar")}
          >
            «
          </button>
          <div className="add">
            <button onClick={() => setMenuOpen((o) => !o)} title="Add">
              ＋
            </button>
            {menuOpen && (
              <div className="menu" onMouseLeave={() => setMenuOpen(false)}>
                <button onClick={() => newHost(null)}>New host…</button>
                <button onClick={newFolder}>New folder…</button>
                <button
                  onClick={() => {
                    setMenuOpen(false);
                    openLocalShell();
                  }}
                >
                  Local shell
                </button>
                <hr />
                <button onClick={() => { setMenuOpen(false); setTransfer("import"); }}>
                  Import sessions…
                </button>
                <button onClick={() => { setMenuOpen(false); setTransfer("export"); }}>
                  Export a backup…
                </button>
                <button onClick={() => { setMenuOpen(false); setTransfer("restore"); }}>
                  Restore a backup…
                </button>
                <hr />
                <button
                  onClick={() => {
                    setMenuOpen(false);
                    setAppearanceOpen(true);
                  }}
                >
                  Appearance…
                </button>
                <hr />
                <button
                  onClick={() => {
                    setMenuOpen(false);
                    update.checkNow();
                  }}
                >
                  {update.checking ? "Checking…" : "Check for updates"}
                </button>
              </div>
            )}
          </div>
        </div>
        <div className="tree">
          {error && <div className="error">{error}</div>}
          {rootFolders.map(renderFolder)}
          {(hostsByFolder.get(null) ?? []).map(renderHost)}
          {workspace && workspace.store.hosts.length === 0 && (
            <div className="empty">
              No saved hosts yet.
              <button className="primary" onClick={() => newHost(null)}>
                Add a host
              </button>
            </div>
          )}
        </div>
        {workspace && (
          <SnippetBar
            snippets={workspace.store.snippets}
            onSend={
              // Only when a terminal is in front — a snippet goes to a shell.
              active && active.kind === "terminal" && !active.exited
                ? (command) => sendInput(active.key, command)
                : null
            }
            onChanged={(snippets) =>
              setWorkspace((w) => (w ? { ...w, store: { ...w.store, snippets } } : w))
            }
          />
        )}

        {logPath && (
          <div className="note recording">
            ⏺ Recording this session to {logPath.split("/").pop()}
          </div>
        )}

        {update.available && (
          <div className="update">
            <strong>BHTerminal {update.available.version} is out.</strong>
            <div className="update-actions">
              <button
                className="primary"
                onClick={() =>
                  api
                    .updateOpen(update.available!.downloadUrl ?? update.available!.pageUrl)
                    .catch((e) => setError(String(e)))
                }
              >
                {update.available.downloadUrl ? "Download" : "See the release"}
              </button>
              <button onClick={update.dismiss}>Later</button>
            </div>
          </div>
        )}
        {update.error && <div className="error">{update.error}</div>}
        {update.upToDate && !update.available && (
          <div className="note">BHTerminal {version} is the latest version.</div>
        )}
        {workspace && !workspace.multiplexing && (
          <div className="note">
            Windows OpenSSH has no connection sharing, so the file browser signs in separately.
          </div>
        )}
      </aside>

      {/* Double-click puts a pane back to the width it shipped with. */}
      {!collapsed.sidebar && (
        <div
          className="pane-divider"
          title="Drag to resize · double-click to reset"
          onMouseDown={() => startDrag("sidebar")}
          onDoubleClick={() => reset("sidebar")}
        />
      )}

      {collapsed.sidebar && (
        <button
          className="rail left"
          title="Show the sessions list"
          onClick={() => toggleCollapsed("sidebar")}
        >
          »
        </button>
      )}

      <main className="main">
        <div className="tabs">
          {tabs.map((t) => {
            const shown = t.panes.find((p) => p.key === t.focused) ?? t.panes[0];
            return (
              <div
                key={t.key}
                className={`tab ${t.key === activeTabKey ? "active" : ""} ${shown.exited ? "exited" : ""}`}
                onClick={() => setActiveTabKey(t.key)}
              >
                <span>{shown.title}</span>
                {t.panes.length > 1 && <span className="split-count">{t.panes.length}</span>}
                <button
                  className="x"
                  onClick={(e) => {
                    e.stopPropagation();
                    closeTab(t.key);
                  }}
                >
                  ✕
                </button>
              </div>
            );
          })}
          {active?.kind === "terminal" && (
            <div className="tab-tools">
              <button title="Split to the right" onClick={() => splitFocused("row")}>
                ⫿
              </button>
              <button title="Split downwards" onClick={() => splitFocused("column")}>
                ⫻
              </button>
              {activeTab && activeTab.panes.length > 1 && (
                <button
                  className={activeTab.broadcast ? "danger-on" : ""}
                  title={
                    activeTab.broadcast
                      ? "Typing reaches every pane in this tab — click to stop"
                      : "Type into every pane in this tab"
                  }
                  onClick={toggleBroadcast}
                >
                  ⇉
                </button>
              )}
              <button
                className={logPath ? "on" : ""}
                title={logPath ? `Logging to ${logPath}` : "Record this session to a file"}
                onClick={toggleLogging}
              >
                ⏺
              </button>
            </div>
          )}
        </div>

        {activeHost && activeHost.connectionType === "ssh" && <TunnelBar host={activeHost} />}

        <div className={`panes ${active?.hostId && active.kind === "terminal" && !active.exited ? "with-files" : ""}`}>
          {tabs.length === 0 && (
            <div className="placeholder">Double-click a host to connect</div>
          )}
          {/* Every pane of every tab stays mounted. Rendering only the active
              one would dispose the others' terminals and lose their
              scrollback — the session itself lives in Rust. */}
          {tabs.map((t) => (
            <div
              key={t.key}
              className={`tab-panes axis-${t.axis} ${t.broadcast && t.panes.length > 1 ? "broadcasting" : ""}`}
              style={{ display: t.key === activeTabKey ? "flex" : "none" }}
            >
              {t.panes.map((p) => (
                <div
                  key={p.key}
                  className={`pane-slot ${
                    t.panes.length > 1 && p.key === t.focused ? "focused" : ""
                  }`}
                  onMouseDown={() => focusPane(t.key, p.key)}
                >
                  {t.panes.length > 1 && (
                    <div className="pane-head">
                      <span>{p.title}</span>
                      <button
                        onClick={(e) => {
                          e.stopPropagation();
                          closePane(t.key, p.key);
                        }}
                      >
                        ✕
                      </button>
                    </div>
                  )}
                  {p.kind === "vnc" && p.hostId ? (
                    <VncPane
                      hostId={p.hostId}
                      hostName={p.title}
                      active={t.key === activeTabKey}
                    />
                  ) : (
                    <TerminalPane
                      paneKey={p.key}
                      hostId={p.hostId}
                      visible={t.key === activeTabKey}
                      mode={mode}
                      font={font}
                      focused={t.key === activeTabKey && p.key === t.focused}
                      onReady={(id) => paneIds.set(p.key, id)}
                      onInput={sendInput}
                      onExit={(key, code) =>
                        patchPane(key, (q) => ({
                          ...q,
                          exited: true,
                          title: `${q.title} — exited${code !== null ? ` (${code})` : ""}`,
                        }))
                      }
                      onTitle={(key, title) => {
                        patchPane(key, (q) => (q.exited ? q : { ...q, title }));
                        // Distro shells publish the working directory in the
                        // title on every prompt, which is why cwd-follow needs
                        // nothing injected into the user's session.
                        const path = parseTitlePath(title);
                        if (path && p.hostId) setCwdByHost((m) => ({ ...m, [p.hostId!]: path }));
                      }}
                    />
                  )}
                </div>
              ))}
            </div>
          ))}
        </div>

        {active?.hostId && active.kind === "terminal" && !active.exited && !collapsed.files && (
          <div
            className="pane-divider files-divider"
            title="Drag to resize · double-click to reset"
            onMouseDown={() => startDrag("files")}
            onDoubleClick={() => reset("files")}
          />
        )}

        {active?.hostId && active.kind === "terminal" && !active.exited && collapsed.files && (
          <button
            className="rail right"
            title="Show the file browser"
            onClick={() => toggleCollapsed("files")}
          >
            «
          </button>
        )}

        {active?.hostId && active.kind === "terminal" && !active.exited && (
          <FilePane
            onCollapse={() => toggleCollapsed("files")}
            key={active.hostId}
            hostId={active.hostId}
            terminalCwd={cwdByHost[active.hostId] ?? null}
            follow={followTerminal}
            onFollowChange={setFollowTerminal}
          />
        )}

        {activeTab?.broadcast && activeTab.panes.length > 1 && (
          <div className="broadcast-banner">
            ⇉ Broadcast is on: what you type goes to all {activeTab.panes.length} panes in this tab.
            <button onClick={toggleBroadcast}>Stop</button>
          </div>
        )}

        {active?.exited && (
          <div className="banner">
            Disconnected. Close the tab, or double-click the host to reconnect.
          </div>
        )}
      </main>

      {appearanceOpen && (
        <AppearanceSheet
          theme={theme}
          font={font}
          onTheme={(t) => {
            setTheme(t);
            setMode(resolvedMode(t));
            // The titlebar is the OS's, not the webview's, so it has to be
            // told separately or a light window keeps a dark titlebar.
            api.setWindowTheme(t === "system" ? null : t).catch(() => {});
          }}
          onFont={(f) => {
            setFont(f);
            saveFont(f);
          }}
          onClose={() => setAppearanceOpen(false)}
        />
      )}

      {ask && (
        <AskModal
          title={ask.title}
          value={ask.value}
          detail={ask.detail}
          confirmLabel={ask.confirmLabel}
          danger={ask.danger}
          onCancel={() => setAsk(null)}
          onSubmit={ask.onSubmit}
        />
      )}

      {editing && workspace && (
        <HostEditor
          host={editing.host}
          isNew={editing.isNew}
          folders={workspace.store.folders}
          hosts={workspace.store.hosts}
          onSaved={(h) => {
            mergeHost(h);
            setEditing(null);
          }}
          onCancel={() => setEditing(null)}
        />
      )}

      {transfer && (
        <TransferSheet
          mode={transfer}
          onChanged={() => {
            // A restore or import can add hundreds of hosts and folders, so
            // the tree is reloaded rather than merged here.
            api.loadWorkspace().then(setWorkspace).catch((e) => setError(String(e)));
          }}
          onClose={() => setTransfer(null)}
        />
      )}
    </div>
  );
}
