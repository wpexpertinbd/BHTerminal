import { useEffect, useMemo, useState } from "react";
import { api, type Host, type HostFolder, type Workspace } from "./api";
import { TerminalPane } from "./TerminalPane";

interface Pane {
  key: string;
  hostId: string | null;
  title: string;
  /** Set when ssh exits unexpectedly: the tab stays so it can be reconnected,
   *  rather than vanishing and taking the error message with it. */
  exited: boolean;
}

let paneSeq = 0;

export default function App() {
  const [workspace, setWorkspace] = useState<Workspace | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [panes, setPanes] = useState<Pane[]>([]);
  const [activeKey, setActiveKey] = useState<string | null>(null);
  const [selected, setSelected] = useState<string | null>(null);
  const [expanded, setExpanded] = useState<Set<string>>(new Set());

  useEffect(() => {
    api.loadWorkspace().then(setWorkspace).catch((e) => setError(String(e)));
  }, []);

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
    // Reuse an open tab for the same host rather than stacking duplicates.
    const existing = panes.find((p) => p.hostId === host.id && !p.exited);
    if (existing) {
      setActiveKey(existing.key);
      return;
    }
    const pane: Pane = { key: `pane-${++paneSeq}`, hostId: host.id, title: host.name, exited: false };
    setPanes((p) => [...p, pane]);
    setActiveKey(pane.key);
  }

  function openLocalShell() {
    const pane: Pane = { key: `pane-${++paneSeq}`, hostId: null, title: "Local shell", exited: false };
    setPanes((p) => [...p, pane]);
    setActiveKey(pane.key);
  }

  const paneIds = useMemo(() => new Map<string, string>(), []);

  function closePane(key: string) {
    const id = paneIds.get(key);
    if (id) api.close(id); // the ONLY thing that ends a session
    paneIds.delete(key);
    setPanes((prev) => {
      const next = prev.filter((p) => p.key !== key);
      setActiveKey((cur) => (cur === key ? next[next.length - 1]?.key ?? null : cur));
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
    const live = panes.some((p) => p.hostId === h.id && !p.exited);
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
            {h.username}@{h.hostname}:{h.port}
          </span>
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
        </div>
        {open && <div className="children">{children.map(renderHost)}</div>}
      </div>
    );
  }

  const active = panes.find((p) => p.key === activeKey) ?? null;

  return (
    <div className="app">
      <aside className="sidebar">
        <div className="sidebar-head">
          <strong>Sessions</strong>
          <button onClick={openLocalShell} title="Open a local shell">＋</button>
        </div>
        <div className="tree">
          {error && <div className="error">{error}</div>}
          {rootFolders.map(renderFolder)}
          {(hostsByFolder.get(null) ?? []).map(renderHost)}
          {workspace && workspace.store.hosts.length === 0 && (
            <div className="empty">No saved hosts yet.</div>
          )}
        </div>
        {workspace && !workspace.multiplexing && (
          <div className="note">
            Windows OpenSSH has no connection sharing, so the file browser signs in separately.
          </div>
        )}
      </aside>

      <main className="main">
        <div className="tabs">
          {panes.map((p) => (
            <div
              key={p.key}
              className={`tab ${p.key === activeKey ? "active" : ""} ${p.exited ? "exited" : ""}`}
              onClick={() => setActiveKey(p.key)}
            >
              <span>{p.title}</span>
              <button
                className="x"
                onClick={(e) => {
                  e.stopPropagation();
                  closePane(p.key);
                }}
              >
                ✕
              </button>
            </div>
          ))}
        </div>

        <div className="panes">
          {panes.length === 0 && (
            <div className="placeholder">Double-click a host to connect</div>
          )}
          {/* Every pane stays mounted. Rendering only the active one would
              dispose the others' terminals and lose their scrollback. */}
          {panes.map((p) => (
            <TerminalPane
              key={p.key}
              paneKey={p.key}
              hostId={p.hostId}
              active={p.key === activeKey}
              onReady={(id) => paneIds.set(p.key, id)}
              onExit={(key, code) =>
                setPanes((prev) =>
                  prev.map((q) => (q.key === key ? { ...q, exited: true, title: `${q.title} — exited${code !== null ? ` (${code})` : ""}` } : q))
                )
              }
              onTitle={(key, title) =>
                setPanes((prev) => prev.map((q) => (q.key === key && !q.exited ? { ...q, title } : q)))
              }
            />
          ))}
        </div>

        {active?.exited && (
          <div className="banner">
            Disconnected. Close the tab, or double-click the host to reconnect.
          </div>
        )}
      </main>
    </div>
  );
}
