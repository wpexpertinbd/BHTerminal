// Finding a host by name, address or user — across every folder at once.
//
// Pure functions only (no React, no Tauri) so the ranking can be tested on its
// own: `node --test app/src/hostSearch.test.ts`.

import type { Host, HostFolder } from "./api";

/** [start, end) character ranges to highlight. */
export type Hits = Array<[number, number]>;

export interface HostMatch {
  host: Host;
  score: number;
  /** "BiswasHost / Clients" — where the host lives, since results are flat. */
  folderPath: string;
  nameHits: Hits;
  addressHits: Hits;
}

/** The folder chain above `folderID`, outermost first. Cycles are cut. */
export function folderPathOf(folderID: string | null, folders: HostFolder[]): string {
  const byId = new Map(folders.map((f) => [f.id, f]));
  const names: string[] = [];
  const seen = new Set<string>();
  let id = folderID;
  while (id && !seen.has(id)) {
    seen.add(id);
    const f = byId.get(id);
    if (!f) break;
    names.unshift(f.name);
    id = f.parentFolderID;
  }
  return names.join(" / ");
}

/** Characters that start a new "word" inside a name or an address. */
const BOUNDARY = /[\s.\-_@:/()[\]]/;

/** How well one query token matches one text: 0 = not at all. */
function tokenScore(text: string, token: string, allowFuzzy: boolean): number {
  if (!text) return 0;
  if (text === token) return 100;
  if (text.startsWith(token)) return 80;
  let at = text.indexOf(token);
  while (at > 0) {
    if (BOUNDARY.test(text[at - 1])) return 60;
    at = text.indexOf(token, at + 1);
  }
  if (text.includes(token)) return 40;
  // Letters in order with gaps ("shfrt" → "shorifart"): only for longer
  // tokens and only on name/address, or every query would match everything.
  // Never for anything with a digit or a dot: "227" would otherwise "match"
  // the IP 65.21.48.27 (2…2…7 in order), and addresses must match literally.
  if (allowFuzzy && token.length >= 3 && !/[\d.]/.test(token) && isSubsequence(token, text)) {
    return 10;
  }
  return 0;
}

function isSubsequence(needle: string, hay: string): boolean {
  let i = 0;
  for (const ch of hay) {
    if (ch === needle[i]) i++;
    if (i === needle.length) return true;
  }
  return false;
}

/** Where the tokens occur in `text` (case-insensitive), merged. */
function hitsIn(text: string, tokens: string[]): Hits {
  const lower = text.toLowerCase();
  // Offsets are found in the lower-cased text and applied to the original.
  // A few characters change length when lower-cased ("İ" → "i̇"), which would
  // shift every range after them: highlight nothing rather than the wrong part.
  if (lower.length !== text.length) return [];
  const ranges: Hits = [];
  for (const t of tokens) {
    let at = lower.indexOf(t);
    while (at >= 0) {
      ranges.push([at, at + t.length]);
      at = lower.indexOf(t, at + t.length);
    }
  }
  ranges.sort((a, b) => a[0] - b[0]);
  const merged: Hits = [];
  for (const r of ranges) {
    const last = merged[merged.length - 1];
    if (last && r[0] <= last[1]) last[1] = Math.max(last[1], r[1]);
    else merged.push([r[0], r[1]]);
  }
  return merged;
}

/** The address line shown under a host's name. */
export function addressOf(h: Host): string {
  return h.connectionType === "vnc"
    ? `VNC ${h.hostname}:${h.port}`
    : `${h.username}@${h.hostname}:${h.port}`;
}

/**
 * Hosts matching `query`, best first. Every whitespace-separated word of the
 * query must match somewhere (name, address, user, port, folder or notes);
 * a host's score is the sum of each word's best field. Name and address
 * count fully, everything else less, so "s3" finds s3.example.com before a
 * host merely filed in a folder called "s3 backups".
 */
export function searchHosts(query: string, hosts: Host[], folders: HostFolder[]): HostMatch[] {
  const tokens = query.toLowerCase().split(/\s+/).filter(Boolean);
  if (tokens.length === 0) return [];
  const out: HostMatch[] = [];
  for (const host of hosts) {
    const folderPath = folderPathOf(host.folderID, folders);
    const fields: Array<[string, number, boolean]> = [
      [host.name.toLowerCase(), 1, true],
      [host.hostname.toLowerCase(), 1, true],
      [`${host.username}@${host.hostname}`.toLowerCase(), 0.9, false],
      [host.username.toLowerCase(), 0.7, false],
      [folderPath.toLowerCase(), 0.5, false],
      [(host.notes ?? "").toLowerCase(), 0.3, false],
    ];
    let total = 0;
    let everyTokenMatched = true;
    for (const t of tokens) {
      let best = 0;
      for (const [text, weight, fuzzy] of fields) {
        best = Math.max(best, tokenScore(text, t, fuzzy) * weight);
      }
      // A port only counts when typed exactly ("2279"), never as a fragment.
      if (String(host.port) === t) best = Math.max(best, 50);
      if (best === 0) {
        everyTokenMatched = false;
        break;
      }
      total += best;
    }
    if (!everyTokenMatched) continue;
    out.push({
      host,
      score: total,
      folderPath,
      nameHits: hitsIn(host.name, tokens),
      addressHits: hitsIn(addressOf(host), tokens),
    });
  }
  out.sort((a, b) => b.score - a.score || a.host.name.localeCompare(b.host.name));
  return out;
}
