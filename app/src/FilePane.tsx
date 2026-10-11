import { useCallback, useEffect, useState } from "react";
import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
import {
  api, formatSize, remoteJoin, remoteParent,
  type Listing, type SftpEntry,
} from "./api";

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

      <div className="files-list">
        {status && <div className="files-status">{status}</div>}
        {listing?.entries.map((e) => (
          <div
            key={e.name}
            className={`frow ${selected === e.name ? "selected" : ""}`}
            onClick={() => setSelected(e.name)}
            onDoubleClick={() => (e.isDir ? load(remoteJoin(listing.path, e.name)) : download(e))}
          >
            <span className="ficon">{e.isDir ? "📁" : e.isSymlink ? "🔗" : "📄"}</span>
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

      <label className="files-follow">
        <input type="checkbox" checked={follow} onChange={(e) => onFollowChange(e.target.checked)} />
        Follow Terminal
      </label>
    </div>
  );
}
