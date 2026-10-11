import { useEffect, useRef, useState } from "react";
import type RFBType from "@novnc/novnc";
import { api } from "./api";

interface Props {
  hostId: string;
  hostName: string;
  active: boolean;
}

/** Renders a VNC desktop with noVNC, which draws RFB straight to a canvas in
 *  this webview. The raw TCP socket a VNC server speaks is reached through
 *  the Rust bridge, which only ever connects to the host this token was
 *  registered for. */
export function VncPane({ hostId, hostName, active }: Props) {
  const holder = useRef<HTMLDivElement | null>(null);
  const rfbRef = useRef<RFBType | null>(null);
  const [status, setStatus] = useState("Connecting…");

  useEffect(() => {
    let disposed = false;
    let rfb: RFBType | null = null;

    (async () => {
      try {
        const url = await api.vncPrepare(hostId);
        // Loaded lazily so noVNC isn't in the bundle for people who never
        // open a desktop.
        const { default: RFB } = await import("@novnc/novnc");
        if (disposed || !holder.current) return;

        rfb = new RFB(holder.current, url);
        rfb.scaleViewport = true;
        rfb.resizeSession = false;
        // Off by default: a malicious or MITM'd server can otherwise read and
        // poison the local clipboard. Matches the per-host setting in the
        // saved session.
        rfb.clipboardPasteFrom = () => {};
        rfbRef.current = rfb;

        rfb.addEventListener("connect", () => setStatus(""));
        rfb.addEventListener("credentialsrequired", () => {
          const password = prompt(`Password for ${hostName}`);
          if (password === null) { rfb?.disconnect(); setStatus("Cancelled."); return; }
          rfb?.sendCredentials({ password });
        });
        rfb.addEventListener("securityfailure", (e: Event) =>
          setStatus(
            `Authentication failed: ${(e as CustomEvent).detail?.reason ?? "rejected"}`
          )
        );
        rfb.addEventListener("disconnect", (e: Event) =>
          setStatus((e as CustomEvent).detail?.clean ? "Disconnected." : "Connection lost.")
        );
      } catch (e) {
        if (!disposed) setStatus(String(e));
      }
    })();

    return () => {
      disposed = true;
      try { rfb?.disconnect(); } catch { /* already gone */ }
    };
  }, [hostId, hostName]);

  return (
    <div className="vnc" style={{ display: active ? "block" : "none" }}>
      {status && <div className="vnc-status">{status}</div>}
      <div className="vnc-canvas" ref={holder} />
    </div>
  );
}
