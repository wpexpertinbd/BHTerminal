/** Imported as the bare package name on purpose: noVNC sets "exports" to a
 *  single string ("./core/rfb.js"), so the package has exactly ONE entry
 *  point and `@novnc/novnc/core/rfb.js` is rejected by the resolver.
 *
 *  noVNC ships no type declarations, so these describe only the surface this
 *  app actually uses. Kept deliberately narrow — a wider guess would be a
 *  fiction the compiler then trusts. */
declare module "@novnc/novnc" {
  interface Credentials {
    password?: string;
    username?: string;
    target?: string;
  }

  export default class RFB extends EventTarget {
    constructor(
      target: HTMLElement,
      urlOrChannel: string,
      options?: { credentials?: Credentials; shared?: boolean; repeaterID?: string }
    );
    /** Scale the remote framebuffer to fit the element. */
    scaleViewport: boolean;
    /** Ask the server to resize to match the element instead. */
    resizeSession: boolean;
    clipboardPasteFrom: (text: string) => void;
    sendCredentials(credentials: Credentials): void;
    disconnect(): void;
  }
}
