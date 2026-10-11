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
  sortOrder: number;
  notes: string;
}

export interface HostFolder {
  id: string;
  name: string;
  parentFolderID: string | null;
  sortOrder: number;
}

export interface SessionStore {
  folders: HostFolder[];
  hosts: Host[];
  snippets: { id: string; name: string; command: string; sortOrder: number }[];
}

export interface Workspace {
  store: SessionStore;
  /** False on Windows: OpenSSH there has no ControlMaster, so SFTP will
   *  authenticate separately. Surfaced so the UI can say so rather than
   *  quietly behaving differently. */
  multiplexing: boolean;
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
};

export { Channel };
