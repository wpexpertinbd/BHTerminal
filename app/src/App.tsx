import { useEffect, useMemo, useState } from "react";
import { api, blankHost, newId, type Host, type HostFolder, type Workspace } from "./api";
import { HostEditor } from "./HostEditor";
import { TransferSheet } from "./TransferSheet";
import { useUpdateCheck } from "./useUpdateCheck";
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

let paneSeq = 0;

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
  const [panes, setPanes] = useState<Pane[]>([]);
  const [activeKey, setActiveKey] = useState<string | null>(null);
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
  const [version, setVersion] = useState<string>("");
  const update = useUpdateCheck(setVersion);

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
    const pane: Pane = {
      key: `pane-${++paneSeq}`,
      kind: host.connectionType === "vnc" ? "vnc" : "terminal",
      hostId: host.id,
      title: host.name,
      exited: false,
    };
    setPanes((p) => [...p, pane]);
    setActiveKey(pane.key);
  }

  function openLocalShell() {
    const pane: Pane = { key: `pane-${++paneSeq}`, kind: "terminal", hostId: null, title: "Local shell", exited: false };
    setPanes((p) => [...p, pane]);
    setActiveKey(pane.key);
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

  async function newFolder() {
    setMenuOpen(false);
    const name = window.prompt("Folder name");
    if (!name?.trim()) return;
    try {
      const saved = await api.saveFolder({
        id: newId(),
        name: name.trim(),
        parentFolderID: null,
        sortOrder: 0,
      });
      setWorkspace((w) =>
        w ? { ...w, store: { ...w.store, folders: [...w.store.folders, saved] } } : w
      );
    } catch (e) {
      setError(String(e));
    }
  }

  async function removeHost(h: Host) {
    if (!window.confirm(`Delete “${h.name}”? Its saved password is deleted too.`)) return;
    try {
      await api.deleteHost(h.id);
      setWorkspace((w) =>
        w ? { ...w, store: { ...w.store, hosts: w.store.hosts.filter((x) => x.id !== h.id) } } : w
      );
      if (selected === h.id) setSelected(null);
    } catch (e) {
      setError(String(e));
    }
  }

  async function removeFolder(f: HostFolder) {
    const inside = (hostsByFolder.get(f.id) ?? []).length;
    const warning = inside
      ? `Delete “${f.name}” and the ${inside} host${inside === 1 ? "" : "s"} inside it?`
      : `Delete “${f.name}”?`;
    if (!window.confirm(warning)) return;
    try {
      await api.deleteFolder(f.id);
      // The delete cascades into subfolders on the Rust side, so reload rather
      // than trying to reproduce that walk here.
      setWorkspace(await api.loadWorkspace());
    } catch (e) {
      setError(String(e));
    }
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

  const active = panes.find((p) => p.key === activeKey) ?? null;
  const activeHost =
    workspace && active?.hostId ? workspace.store.hosts.find((h) => h.id === active.hostId) : undefined;

  // SFTP rides the terminal's connection, so drop sessions whose host has no
  // tab left open.
  useEffect(() => {
    const keep = Array.from(new Set(panes.map((p) => p.hostId).filter(Boolean) as string[]));
    api.sftpPrune(keep).catch(() => {});
  }, [panes]);

  return (
    <div className="app">
      <aside className="sidebar">
        <div className="sidebar-head">
          <strong>Sessions</strong>
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
                ? (command) => {
                    const id = paneIds.get(active.key);
                    if (id) api.write(id, command).catch((e) => setError(String(e)));
                  }
                : null
            }
            onChanged={(snippets) =>
              setWorkspace((w) => (w ? { ...w, store: { ...w.store, snippets } } : w))
            }
          />
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

        {activeHost && activeHost.connectionType === "ssh" && <TunnelBar host={activeHost} />}

        <div className={`panes ${active?.hostId && active.kind === "terminal" && !active.exited ? "with-files" : ""}`}>
          {panes.length === 0 && (
            <div className="placeholder">Double-click a host to connect</div>
          )}
          {/* Every pane stays mounted. Rendering only the active one would
              dispose the others' terminals and lose their scrollback. */}
          {panes.map((p) =>
            p.kind === "vnc" && p.hostId ? (
              <VncPane
                key={p.key}
                hostId={p.hostId}
                hostName={p.title}
                active={p.key === activeKey}
              />
            ) : (
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
              onTitle={(key, title) => {
                setPanes((prev) => prev.map((q) => (q.key === key && !q.exited ? { ...q, title } : q)));
                // Distro shells publish the working directory in the title on
                // every prompt, which is why cwd-follow needs nothing injected
                // into the user's session.
                const path = parseTitlePath(title);
                if (path && p.hostId) setCwdByHost((m) => ({ ...m, [p.hostId!]: path }));
              }}
            />
            )
          )}
        </div>

        {active?.hostId && active.kind === "terminal" && !active.exited && (
          <FilePane
            key={active.hostId}
            hostId={active.hostId}
            terminalCwd={cwdByHost[active.hostId] ?? null}
            follow={followTerminal}
            onFollowChange={setFollowTerminal}
          />
        )}

        {active?.exited && (
          <div className="banner">
            Disconnected. Close the tab, or double-click the host to reconnect.
          </div>
        )}
      </main>

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
