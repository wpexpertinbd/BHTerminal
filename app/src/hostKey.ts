// Recognising ssh's "REMOTE HOST IDENTIFICATION HAS CHANGED!" refusal in a
// session's output, so the app can offer to trust the new key instead of
// leaving the user to edit known_hosts by hand.
//
// Pure functions only: `node --test app/src/hostKey.test.ts`.

export interface HostKeyChange {
  /** The server as ssh named it: "167.86.123.107" or "[example.com]:2279". */
  reported: string;
  /** "SHA256:…" of the key the server sent now, when ssh printed it. */
  fingerprint: string | null;
}

/** Colour and cursor codes, and Windows' title sequences, out of the way. */
function plain(text: string): string {
  return text
    .replace(/\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)/g, "")
    .replace(/\x1b\[[0-?]*[ -/]*[@-~]/g, "")
    .replace(/\r/g, "");
}

/**
 * The change ssh refused to connect over, or null.
 *
 * Only the shape ssh itself leaves behind counts: the banner, its own
 * "Host key for X has changed" line, the session ENDING on "Host key
 * verification failed." and ssh's exit code 255. A logged-in session that
 * merely prints the words and carries on, or ends some other way, does not
 * get a "trust the new key" offer.
 */
export function findHostKeyChange(output: string, exitCode: number | null): HostKeyChange | null {
  if (exitCode !== 255) return null;
  const text = plain(output);
  const lines = text.split("\n").map((l) => l.trim()).filter(Boolean);
  if (lines[lines.length - 1] !== "Host key verification failed.") return null;
  if (!/REMOTE HOST IDENTIFICATION HAS CHANGED/.test(text)) return null;
  const host = /Host key for (\S+) has changed/.exec(text);
  if (!host) return null;
  const fp = /(SHA256:[A-Za-z0-9+/=]+)/.exec(text);
  return { reported: host[1], fingerprint: fp ? fp[1] : null };
}
