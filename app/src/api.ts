import { invoke, Channel } from "@tauri-apps/api/core";

export type AuthMethod =
  | { password: Record<string, never> }
  | { privateKey: { path: string } }
  | { agent: Record<string, never> };

export interface Host {
  id: string;
  name: string;
  hostname: string;
  port: number;
  username: string;
  authMethod: AuthMethod;
  connectionType: "ssh" | "vnc";
  folderID: string | null;
  jumpHostID: string | null;
  tunnels: TunnelRule[];
  sortOrder: number;
  notes: string;
}

export interface HostFolder {
  id: string;
  name: string;
  parentFolderID: string | null;
  sortOrder: number;
}

export interface Snippet {
  id: string;
  name: string;
  command: string;
  sortOrder: number;
}

export type TunnelKind = "local" | "remote" | "dynamic";

export interface TunnelRule {
  id: string;
  name: string;
  kind: TunnelKind;
  listenHost: string;
  listenPort: number;
  destHost: string;
  destPort: number;
  enabled: boolean;
}

export interface TunnelStatus {
  ruleId: string;
  running: boolean;
  /** Set once it has stopped: ssh's own reason. */
  message: string | null;
}

export interface SessionStore {
  folders: HostFolder[];
  hosts: Host[];
  snippets: Snippet[];
}

export interface Workspace {
  store: SessionStore;
  /** False on Windows: OpenSSH there has no ControlMaster, so SFTP will
   *  authenticate separately. Surfaced so the UI can say so rather than
   *  quietly behaving differently. */
  multiplexing: boolean;
}

export interface RestoreSummary {
  hostsAdded: number;
  hostsUpdated: number;
  foldersAdded: number;
  foldersUpdated: number;
  snippetsAdded: number;
  snippetsUpdated: number;
  secretsRestored: number;
}

export interface RestoreReport extends RestoreSummary {
  /** Hosts whose key file isn't on this computer — expected when a backup
   *  moves between machines, and worth saying before a connect fails. */
  missingKeys: string[];
}

export interface ImportPreview {
  source: string;
  hosts: number;
  warnings: string[];
}

export interface ImportResult {
  source: string;
  hostsAdded: number;
  foldersAdded: number;
  skipped: number;
  passwordsSaved: number;
  warnings: string[];
}

export interface UpdateQuery {
  url: string;
  currentVersion: string;
  releasesPage: string;
}

export interface Available {
  version: string;
  downloadUrl: string | null;
  downloadName: string | null;
  pageUrl: string;
}

export type PtyEvent =
  | { type: "output"; data: string }
  | { type: "exit"; code: number | null };

/** The pty carries bytes, not text — a UTF-8 sequence can straddle two reads,
 *  so the Rust side base64s whole chunks and xterm.js is fed the raw bytes. */
export function decodeChunk(b64: string): Uint8Array {
  const bin = atob(b64);
  const out = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
  return out;
}

function encodeChunk(text: string): string {
  const bytes = new TextEncoder().encode(text);
  let bin = "";
  for (const b of bytes) bin += String.fromCharCode(b);
  return btoa(bin);
}

export interface SftpEntry {
  name: string;
  isDir: boolean;
  isSymlink: boolean;
  size: number | null;
  permissions: number | null;
  mtime: number | null;
}

export interface Listing {
  path: string;
  /** The directory the session logged into — what a title's "~" expands against. */
  home: string;
  entries: SftpEntry[];
}

export const api = {
  loadWorkspace: () => invoke<Workspace>("load_workspace"),
  saveWorkspace: (store: SessionStore) => invoke<void>("save_workspace", { store }),

  openTerminal: (hostId: string, cols: number, rows: number, onEvent: Channel<PtyEvent>) =>
    invoke<string>("open_terminal", { hostId, cols, rows, onEvent }),

  openLocalShell: (cols: number, rows: number, onEvent: Channel<PtyEvent>) =>
    invoke<string>("open_local_shell", { cols, rows, onEvent }),

  write: (paneId: string, text: string) =>
    invoke<void>("write_terminal", { paneId, data: encodeChunk(text) }),

  resize: (paneId: string, cols: number, rows: number) =>
    invoke<void>("resize_terminal", { paneId, cols, rows }),

  close: (paneId: string) => invoke<void>("close_terminal", { paneId }),

  sftpConnect: (hostId: string) => invoke<Listing>("sftp_connect", { hostId }),
  sftpList: (hostId: string, path: string) => invoke<Listing>("sftp_list", { hostId, path }),
  sftpMkdir: (hostId: string, path: string, name: string) =>
    invoke<void>("sftp_mkdir", { hostId, path, name }),
  sftpRename: (hostId: string, from: string, to: string) =>
    invoke<void>("sftp_rename", { hostId, from, to }),
  sftpDelete: (hostId: string, path: string, isDir: boolean) =>
    invoke<void>("sftp_delete", { hostId, path, isDir }),
  sftpDownload: (hostId: string, remote: string, local: string) =>
    invoke<number>("sftp_download", { hostId, remote, local }),
  sftpUpload: (hostId: string, local: string, remoteDir: string) =>
    invoke<number>("sftp_upload", { hostId, local, remoteDir }),
  /** Writes a local file to an EXACT remote path — used when saving an edit
   *  back, where the name must not drift. */
  sftpUploadTo: (hostId: string, local: string, remote: string) =>
    invoke<number>("sftp_upload_to", { hostId, local, remote }),
  sftpPrune: (keep: string[]) => invoke<void>("sftp_prune", { keep }),

  vncPrepare: (hostId: string) => invoke<string>("vnc_prepare", { hostId }),

  /** `secret` null leaves a stored credential alone; "" deletes it. */
  saveHost: (host: Host, secret: string | null) =>
    invoke<Host>("save_host", { host, secret }),
  /** Only whether something is stored — the secret itself never comes back. */
  hostHasSecret: (hostId: string) => invoke<boolean>("host_has_secret", { hostId }),
  deleteHost: (hostId: string) => invoke<void>("delete_host", { hostId }),
  saveFolder: (folder: HostFolder) => invoke<HostFolder>("save_folder", { folder }),
  deleteFolder: (folderId: string) => invoke<void>("delete_folder", { folderId }),

  /** Returns how many secrets went into the archive. */
  backupExport: (path: string, includeSecrets: boolean, passphrase: string | null) =>
    invoke<number>("backup_export", { path, includeSecrets, passphrase }),
  backupIsEncrypted: (path: string) => invoke<boolean>("backup_is_encrypted", { path }),
  backupRestore: (path: string, passphrase: string | null) =>
    invoke<RestoreReport>("backup_restore", { path, passphrase }),
  importPreview: (path: string) => invoke<ImportPreview>("import_preview", { path }),
  importApply: (path: string) => invoke<ImportResult>("import_apply", { path }),

  /** Packs entries of `dir` into one archive there; returns its name. */
  sftpCompress: (hostId: string, dir: string, names: string[], format: "zip" | "tar.gz") =>
    invoke<string>("sftp_compress", { hostId, dir, names, format }),
  sftpExtract: (hostId: string, dir: string, name: string) =>
    invoke<void>("sftp_extract", { hostId, dir, name }),

  /** Where a remote file is staged locally while it is being edited. */
  editScratchPath: (hostId: string, remote: string) =>
    invoke<string>("edit_scratch_path", { hostId, remote }),
  readTextFile: (path: string) => invoke<string>("read_text_file", { path }),
  writeTextFile: (path: string, contents: string) =>
    invoke<void>("write_text_file", { path, contents }),
  fileMtime: (path: string) => invoke<number>("file_mtime", { path }),
  /** `app` names an application (macOS); null uses the system default. */
  openInApp: (path: string, app: string | null) => invoke<void>("open_in_app", { path, app }),
  discardScratch: (path: string) => invoke<void>("discard_scratch", { path }),

  /** null hands the native window chrome back to the system. */
  setWindowTheme: (theme: string | null) => invoke<void>("set_window_theme", { theme }),

  startLogging: (paneId: string, path: string) =>
    invoke<void>("start_logging", { paneId, path }),
  stopLogging: (paneId: string) => invoke<void>("stop_logging", { paneId }),
  loggingPath: (paneId: string) => invoke<string | null>("logging_path", { paneId }),
  suggestedLogName: (hostId: string | null) =>
    invoke<string>("suggested_log_name", { hostId }),

  saveSnippet: (snippet: Snippet) => invoke<Snippet>("save_snippet", { snippet }),
  deleteSnippet: (snippetId: string) => invoke<void>("delete_snippet", { snippetId }),

  tunnelStart: (hostId: string, ruleId: string) =>
    invoke<void>("tunnel_start", { hostId, ruleId }),
  tunnelStop: (ruleId: string) => invoke<void>("tunnel_stop", { ruleId }),
  tunnelStatus: (ruleIds: string[]) => invoke<TunnelStatus[]>("tunnel_status", { ruleIds }),

  updateQuery: () => invoke<UpdateQuery>("update_query"),
  /** null = nothing newer. A FAILED check throws, and must never be shown as
   *  "up to date" — that hides the real state. */
  updateEvaluate: (body: string) => invoke<Available | null>("update_evaluate", { body }),
  updateOpen: (url: string) => invoke<void>("update_open", { url }),
};

/** Remote paths are always "/"-separated whatever the local platform is. */
export function remoteJoin(base: string, name: string): string {
  return base.endsWith("/") ? base + name : `${base}/${name}`;
}

export function remoteParent(path: string): string {
  const trimmed = path.replace(/\/+$/, "");
  const i = trimmed.lastIndexOf("/");
  return i <= 0 ? "/" : trimmed.slice(0, i);
}

export function formatSize(bytes: number | null): string {
  if (bytes === null) return "";
  const units = ["B", "KB", "MB", "GB", "TB"];
  let v = bytes, u = 0;
  while (v >= 1024 && u < units.length - 1) { v /= 1024; u++; }
  return `${u === 0 ? v : v.toFixed(1)} ${units[u]}`;
}

export { Channel };

/** Swift wrote UUIDs uppercased and the Rust side keeps that, so new ids made
 *  here must match or the two spellings would not compare equal. */
export function newId(): string {
  return crypto.randomUUID().toUpperCase();
}

export function blankHost(folderID: string | null): Host {
  return {
    id: newId(),
    name: "",
    hostname: "",
    port: 22,
    username: "",
    authMethod: { password: {} },
    connectionType: "ssh",
    folderID,
    jumpHostID: null,
    tunnels: [],
    sortOrder: 0,
    notes: "",
  };
}

export function authKind(m: AuthMethod): "password" | "privateKey" | "agent" {
  if ("privateKey" in m) return "privateKey";
  if ("agent" in m) return "agent";
  return "password";
}

export function blankTunnel(): TunnelRule {
  return {
    id: newId(),
    name: "",
    kind: "local",
    // Loopback, not 0.0.0.0: a forward bound to every interface exposes the
    // remote service to the whole network the laptop is on.
    listenHost: "127.0.0.1",
    listenPort: 8080,
    destHost: "127.0.0.1",
    destPort: 80,
    enabled: true,
  };
}
