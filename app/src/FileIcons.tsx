/** Folder and file glyphs.
 *
 *  Drawn rather than emoji: 📁 renders as a flat grey glyph in the system
 *  font on macOS, which is what made the file list look lifeless. These
 *  carry the colours people expect — a yellow folder, a pale document, a
 *  tinted archive — and stay legible in both themes. */

export function FolderIcon() {
  return (
    <svg viewBox="0 0 16 16" width="15" height="15" aria-hidden="true">
      <path
        d="M1.5 4.2c0-.66.54-1.2 1.2-1.2h3.07c.4 0 .78.2 1 .54l.6.9h6.03c.66 0 1.2.54 1.2 1.2v6.16c0 .66-.54 1.2-1.2 1.2H2.7c-.66 0-1.2-.54-1.2-1.2z"
        fill="#f3b94d"
      />
      <path
        d="M1.5 6.4h13.1v5.4c0 .66-.54 1.2-1.2 1.2H2.7c-.66 0-1.2-.54-1.2-1.2z"
        fill="#ffcf6b"
      />
    </svg>
  );
}

export function FileIcon({ name }: { name: string }) {
  const lower = name.toLowerCase();
  const archive = /\.(zip|tar|gz|tgz|bz2|tbz|xz|txz|7z|rar)$/.test(lower);
  const code = /\.(sh|bash|zsh|py|rb|pl|php|js|ts|tsx|jsx|rs|go|c|h|cpp|java|sql|html|css|scss)$/.test(lower);
  const config = /(^\.|\.(conf|cfg|ini|env|ya?ml|toml|json|service)$)/.test(lower);
  const fill = archive ? "#c9a227" : code ? "#5a9bd8" : config ? "#8a8f98" : "#b9bec7";
  const page = archive ? "#f6e6a8" : code ? "#dbe9f8" : "#eceef1";
  return (
    <svg viewBox="0 0 16 16" width="15" height="15" aria-hidden="true">
      <path d="M3.6 1.8h5.3l3.5 3.5v8.9c0 .55-.45 1-1 1H3.6c-.55 0-1-.45-1-1V2.8c0-.55.45-1 1-1z" fill={page} />
      <path d="M8.9 1.8 12.4 5.3H9.9c-.55 0-1-.45-1-1z" fill={fill} />
      <path d="M3.6 1.8h5.3l3.5 3.5v8.9c0 .55-.45 1-1 1H3.6c-.55 0-1-.45-1-1V2.8c0-.55.45-1 1-1z" fill="none" stroke={fill} strokeWidth="1" />
    </svg>
  );
}

export function LinkIcon() {
  return (
    <svg viewBox="0 0 16 16" width="15" height="15" aria-hidden="true">
      <path
        d="M6.4 9.6 9.6 6.4M6.9 4.6l1.2-1.2a2.7 2.7 0 1 1 3.8 3.8l-1.2 1.2M9.1 11.4l-1.2 1.2a2.7 2.7 0 0 1-3.8-3.8l1.2-1.2"
        fill="none"
        stroke="#5a9bd8"
        strokeWidth="1.4"
        strokeLinecap="round"
      />
    </svg>
  );
}
