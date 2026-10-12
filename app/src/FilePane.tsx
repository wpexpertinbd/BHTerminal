import { useCallback, useEffect, useRef, useState } from "react";
import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
import {
  api, formatSize, remoteJoin, remoteParent,
  type Listing, type SftpEntry,
} from "./api";
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
}

export function FilePane({ hostId, terminalCwd, follow, onFollowChange }: Props) {
  const [listing, setListing] = useState<Listing | null>(null);
  const [status, setStatus] = useState("Connecting…");
  const [busy, setBusy] = useState(false);
  const [selected, setSelected] = useState<string | null>(null);
  const [menu, setMenu] = useState<{ x: number; y: number; items: MenuAction[] } | null>(null);
  const [editing, setEditing] = useState<{ remote: string; name: string } | null>(null);
  /** Files handed to another application, watched so a save over there comes
   *  back here. Keyed by the local scratch path. */
  const watched = useRef(new Map<string, { remote: string; mtime: number }>());

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
  useEffect(() => {
    let cancelled = false;
    setStatus("Connecting…");
    api
      .sftpConnect(hostId)
      .then((l) => {
        if (!cancelled) { setListing(l); setStatus(""); }
      })
      .catch((e) => !cancelled && setStatus(String(e)));
    return () => { cancelled = true; };
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

  async function makeFolder() {
    if (!listing) return;
    const name = prompt("New folder name");
    if (!name) return;
    try {
      await api.sftpMkdir(hostId, listing.path, name);
      await load(listing.path);
    } catch (e) { setStatus(String(e)); }
  }

  async function rename(entry: SftpEntry) {
    if (!listing) return;
    const next = prompt("Rename to", entry.name);
    if (!next || next === entry.name) return;
    try {
      await api.sftpRename(hostId, remoteJoin(listing.path, entry.name), remoteJoin(listing.path, next));
      await load(listing.path);
    } catch (e) { setStatus(String(e)); }
  }

  async function remove(entry: SftpEntry) {
    if (!listing) return;
    if (!confirm(`Delete “${entry.name}”? This cannot be undone.`)) return;
    try {
      await api.sftpDelete(hostId, remoteJoin(listing.path, entry.name), entry.isDir);
      await load(listing.path);
    } catch (e) { setStatus(String(e)); }
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
        <button disabled={!listing} onClick={() => listing && load(listing.path)} title="Refresh">↻</button>
        <button disabled={!listing} onClick={makeFolder} title="New folder">＋</button>
        <button disabled={!listing} onClick={upload} title="Upload…">⤒</button>
        {busy && <span className="spin">…</span>}
      </div>

      <div className="files-path" title={listing?.path}>{listing?.path ?? ""}</div>

      <div className="files-list" onContextMenu={backgroundMenu}>
        {status && <div className="files-status">{status}</div>}
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
