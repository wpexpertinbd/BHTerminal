import { useCallback, useEffect, useRef, useState } from "react";
import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
import {
  api, formatSize, remoteJoin, remoteParent,
  type Listing, type SftpEntry,
} from "./api";
import { AskModal } from "./AskModal";
import { ContextMenu, type MenuAction } from "./ContextMenu";
import { FileEditor } from "./FileEditor";
import { FileIcon, FolderIcon, LinkIcon } from "./FileIcons";

/** Names BHTerminal knows how to unpack on the server. */
const ARCHIVE = /\.(zip|tar|tar\.gz|tgz|tar\.bz2|tbz|tbz2|tar\.xz|txz|gz)$/i;

interface Props {
  hostId: string;
  /** Latest directory the terminal reported, via its window title. */
  terminalCwd: string | null;
  follow: boolean;
  onFollowChange: (on: boolean) => void;
  /** Hides the pane, leaving a rail to bring it back. */
  onCollapse: () => void;
  /** How the host logs in — decides what "connecting" really means. */
  authKind?: "password" | "privateKey" | "agent";
  /** True where the file pane rides the terminal's own login (macOS/Linux);
   *  false on Windows, where it has to sign in by itself. */
  sharesLogin?: boolean;
}

export function FilePane({
  hostId, terminalCwd, follow, onFollowChange, onCollapse, authKind, sharesLogin = true,
}: Props) {
  // A password host never dials the server from here: it waits for the
  // terminal's login, which can take as long as a person takes to type.
  const connectingText =
    authKind === "password" && sharesLogin ? "Waiting for the terminal login…" : "Connecting…";
  const [listing, setListing] = useState<Listing | null>(null);
  const [status, setStatus] = useState(connectingText);
  const [failed, setFailed] = useState(false);
  const [busy, setBusy] = useState(false);
  const [selected, setSelected] = useState<string | null>(null);
  const [menu, setMenu] = useState<{ x: number; y: number; items: MenuAction[] } | null>(null);
  const [editing, setEditing] = useState<{ remote: string; name: string } | null>(null);
  /** Files handed to another application, watched so a save over there comes
   *  back here. Keyed by the local scratch path. */
  const watched = useRef(new Map<string, { remote: string; mtime: number }>());
  /** The question on screen, if any — see AskModal for why these are not
   *  window.prompt/confirm. */
  const [ask, setAsk] = useState<
    | null
    | {
        title: string;
        value?: string;
        detail?: string;
        confirmLabel?: string;
        danger?: boolean;
        onSubmit: (value: string) => void;
      }
  >(null);

  const load = useCallback(
    async (path: string) => {
      setBusy(true);
      try {
        const l = await api.sftpList(hostId, path);
        setListing(l);
        setStatus("");
      } catch (e) {
        setStatus(String(e));
      } finally {
        setBusy(false);
      }
    },
    [hostId]
  );

  // One session per host, opened once. Reconnecting on every mount is what
  // made the old build spend its time reconnecting instead of browsing.
  // `connectSeq` makes only the newest attempt count, so a stale answer from an
  // earlier attempt (or an unmounted pane) can never overwrite a newer state.
  const connectSeq = useRef(0);
  const connect = useCallback(() => {
    const mine = ++connectSeq.current;
    setFailed(false);
    setStatus(connectingText);
    api
      .sftpConnect(hostId)
      .then((l) => {
        if (connectSeq.current !== mine) return;
        setListing(l);
        setStatus("");
      })
      .catch((e) => {
        if (connectSeq.current !== mine) return;
        setStatus(String(e));
        setFailed(true);
      });
  }, [hostId, connectingText]);

  useEffect(() => {
    connect();
    return () => {
      connectSeq.current++;
      // Don't leave a thread waiting on a login nobody is watching for.
      api.sftpCancelWait(hostId).catch(() => {});
    };
    // Only when the host changes; `connect` is rebuilt for the same host by a
    // prop change and must not restart a login that is already in progress.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [hostId]);

  // Follow the terminal. "~" is expanded against the directory the SFTP
  // session logged into, because only the remote knows its own home.
  useEffect(() => {
    if (!follow || !terminalCwd || !listing) return;
    let target = terminalCwd;
    if (target === "~") target = listing.home;
    else if (target.startsWith("~/")) target = remoteJoin(listing.home, target.slice(2));
    if (target !== listing.path) load(target);
  }, [follow, terminalCwd, listing, load]);

  async function download(entry: SftpEntry) {
    if (!listing) return;
    const local = await saveDialog({ defaultPath: entry.name });
    if (!local) return;
    setBusy(true);
    try {
      const n = await api.sftpDownload(hostId, remoteJoin(listing.path, entry.name), local);
      setStatus(`Downloaded ${formatSize(n)}`);
    } catch (e) { setStatus(String(e)); } finally { setBusy(false); }
  }

  async function upload() {
    if (!listing) return;
    const picked = await openDialog({ multiple: false });
    if (!picked || Array.isArray(picked)) return;
    setBusy(true);
    try {
      await api.sftpUpload(hostId, picked, listing.path);
      await load(listing.path);
    } catch (e) { setStatus(String(e)); } finally { setBusy(false); }
  }

  function makeFile() {
    if (!listing) return;
    setAsk({
      title: "New file",
      value: "",
      detail: "It is created empty, and opens for editing.",
      confirmLabel: "Create",
      onSubmit: async (name) => {
        setAsk(null);
        try {
          await api.sftpCreateFile(hostId, listing.path, name);
          await load(listing.path);
          // Straight into the editor: an empty file is only ever a step
          // towards having something in it.
          setEditing({ remote: remoteJoin(listing.path, name), name });
        } catch (e) { setStatus(String(e)); }
      },
    });
  }

  function makeFolder() {
    if (!listing) return;
    setAsk({
      title: "New folder",
      value: "",
      confirmLabel: "Create",
      onSubmit: async (name) => {
        setAsk(null);
        try {
          await api.sftpMkdir(hostId, listing.path, name);
          await load(listing.path);
        } catch (e) { setStatus(String(e)); }
      },
    });
  }

  function rename(entry: SftpEntry) {
    if (!listing) return;
    setAsk({
      title: `Rename “${entry.name}”`,
      value: entry.name,
      confirmLabel: "Rename",
      onSubmit: async (next) => {
        setAsk(null);
        if (next === entry.name) return;
        try {
          await api.sftpRename(
            hostId,
            remoteJoin(listing.path, entry.name),
            remoteJoin(listing.path, next)
          );
          await load(listing.path);
        } catch (e) { setStatus(String(e)); }
      },
    });
  }

  function remove(entry: SftpEntry) {
    if (!listing) return;
    setAsk({
      title: `Delete “${entry.name}”?`,
      detail: entry.isDir
        ? "The folder and everything in it is removed from the server. This cannot be undone."
        : "It is removed from the server. This cannot be undone.",
      confirmLabel: "Delete",
      danger: true,
      onSubmit: async () => {
        setAsk(null);
        try {
          await api.sftpDelete(hostId, remoteJoin(listing.path, entry.name), entry.isDir);
          await load(listing.path);
        } catch (e) { setStatus(String(e)); }
      },
    });
  }

  /** Opens a remote file in another application, then watches the staged
   *  copy: when that app saves, the change is uploaded back. This is what
   *  makes "edit it in my own editor" work at all — without the watch the
   *  edit would sit in a cache directory and never reach the server. */
  async function openWith(entry: SftpEntry, app: string | null) {
    if (!listing) return;
    const remote = remoteJoin(listing.path, entry.name);
    setBusy(true);
    try {
      const local = await api.editScratchPath(hostId, remote);
      await api.sftpDownload(hostId, remote, local);
      await api.openInApp(local, app);
      watched.current.set(local, { remote, mtime: await api.fileMtime(local) });
      setStatus(`Opened ${entry.name} — saving there uploads it back`);
    } catch (e) {
      setStatus(String(e));
    } finally {
      setBusy(false);
    }
  }

  // Polled rather than watched with a file-system API: an editor save is a
  // human-speed event, and polling needs no extra dependency and behaves the
  // same on every platform.
  useEffect(() => {
    if (watched.current.size === 0) return;
    const timer = setInterval(async () => {
      for (const [local, info] of Array.from(watched.current.entries())) {
        try {
          const mtime = await api.fileMtime(local);
          if (mtime === info.mtime) continue;
          watched.current.set(local, { ...info, mtime });
          await api.sftpUploadTo(hostId, local, info.remote);
          setStatus(`Saved ${info.remote.split("/").pop()} to the server`);
          if (listing) load(listing.path);
        } catch {
          // The file may have been moved or deleted locally; stop watching
          // it rather than reporting the same failure every two seconds.
          watched.current.delete(local);
        }
      }
    }, 2000);
    return () => clearInterval(timer);
  }, [hostId, listing, load, menu]);

  async function compress(entry: SftpEntry, format: "zip" | "tar.gz") {
    if (!listing) return;
    setBusy(true);
    setStatus(`Compressing ${entry.name}…`);
    try {
      const archive = await api.sftpCompress(hostId, listing.path, [entry.name], format);
      setStatus(`Created ${archive}`);
      await load(listing.path);
    } catch (e) {
      setStatus(String(e));
    } finally {
      setBusy(false);
    }
  }

  async function extract(entry: SftpEntry) {
    if (!listing) return;
    setBusy(true);
    setStatus(`Extracting ${entry.name}…`);
    try {
      await api.sftpExtract(hostId, listing.path, entry.name);
      setStatus(`Extracted ${entry.name}`);
      await load(listing.path);
    } catch (e) {
      setStatus(String(e));
    } finally {
      setBusy(false);
    }
  }

  function rowMenu(event: React.MouseEvent, entry: SftpEntry): void {
    event.preventDefault();
    event.stopPropagation();
    setSelected(entry.name);
    const remote = listing ? remoteJoin(listing.path, entry.name) : entry.name;
    const items: MenuAction[] = entry.isDir
      ? [
          { label: "Open", run: () => listing && load(remote) },
          { divider: true },
          { label: "Compress to .tar.gz", run: () => compress(entry, "tar.gz") },
          { label: "Compress to .zip", run: () => compress(entry, "zip") },
        ]
      : [
          { label: "Edit here", run: () => setEditing({ remote, name: entry.name }) },
          { label: "Open with the default app", run: () => openWith(entry, null) },
          { label: "Open with BHNotepad", run: () => openWith(entry, "BHNotepad") },
          { divider: true },
          { label: "Download…", run: () => download(entry) },
          { divider: true },
          { label: "Compress to .tar.gz", run: () => compress(entry, "tar.gz") },
          { label: "Compress to .zip", run: () => compress(entry, "zip") },
          {
            label: "Extract here",
            run: () => extract(entry),
            disabled: !ARCHIVE.test(entry.name),
          },
        ];
    items.push(
      { divider: true },
      { label: "Copy path", run: () => navigator.clipboard?.writeText(remote).catch(() => {}) },
      { label: "Rename…", run: () => rename(entry) },
      { label: "Delete", run: () => remove(entry), danger: true }
    );
    setMenu({ x: event.clientX, y: event.clientY, items });
  }

  function backgroundMenu(event: React.MouseEvent): void {
    event.preventDefault();
    if (!listing) return;
    setMenu({
      x: event.clientX,
      y: event.clientY,
      items: [
        { label: "New file…", run: makeFile },
        { label: "New folder…", run: makeFolder },
        { label: "Upload a file…", run: upload },
        { divider: true },
        { label: "Refresh", run: () => load(listing.path) },
        { label: "Copy folder path", run: () => navigator.clipboard?.writeText(listing.path).catch(() => {}) },
      ],
    });
  }

  return (
    <div className="files">
      <div className="files-bar">
        <button disabled={!listing || listing.path === "/"} onClick={() => listing && load(remoteParent(listing.path))} title="Up">↑</button>
        <button
          disabled={!listing && !failed}
          onClick={() => (listing ? load(listing.path) : connect())}
          title={listing ? "Refresh" : "Try again"}
        >
          ↻
        </button>
        <button disabled={!listing} onClick={makeFolder} title="New folder">＋</button>
        <button disabled={!listing} onClick={upload} title="Upload…">⤒</button>
        {busy && <span className="spin">…</span>}
        <button className="collapse" onClick={onCollapse} title="Hide the file browser">
          »
        </button>
      </div>

      <div className="files-path" title={listing?.path}>{listing?.path ?? ""}</div>

      <div className="files-list" onContextMenu={backgroundMenu}>
        {status && (
          <div className="files-status">
            {status}
            {failed && !listing && (
              <div>
                <button className="retry" onClick={connect}>
                  Try again
                </button>
              </div>
            )}
          </div>
        )}
        {listing?.entries.map((e) => (
          <div
            key={e.name}
            className={`frow ${selected === e.name ? "selected" : ""}`}
            onClick={() => setSelected(e.name)}
            onContextMenu={(ev) => rowMenu(ev, e)}
            onDoubleClick={() =>
              e.isDir
                ? load(remoteJoin(listing.path, e.name))
                : setEditing({ remote: remoteJoin(listing.path, e.name), name: e.name })
            }
          >
            <span className="ficon">
              {e.isDir ? <FolderIcon /> : e.isSymlink ? <LinkIcon /> : <FileIcon name={e.name} />}
            </span>
            <span className="fname">{e.name}</span>
            <span className="fsize">{e.isDir ? "" : formatSize(e.size)}</span>
            <span className="fact">
              <button onClick={(ev) => { ev.stopPropagation(); rename(e); }} title="Rename">✎</button>
              <button onClick={(ev) => { ev.stopPropagation(); remove(e); }} title="Delete">🗑</button>
            </span>
          </div>
        ))}
        {listing && listing.entries.length === 0 && !status && (
          <div className="files-status">Empty folder</div>
        )}
      </div>

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

      {menu && <ContextMenu x={menu.x} y={menu.y} items={menu.items} onClose={() => setMenu(null)} />}

      {editing && (
        <FileEditor
          hostId={hostId}
          remote={editing.remote}
          name={editing.name}
          onClose={() => setEditing(null)}
          onSaved={() => listing && load(listing.path)}
        />
      )}

      <label className="files-follow">
        <input type="checkbox" checked={follow} onChange={(e) => onFollowChange(e.target.checked)} />
        Follow Terminal
      </label>
    </div>
  );
}
