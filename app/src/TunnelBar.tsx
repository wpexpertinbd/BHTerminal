import { useEffect, useState } from "react";
import { api, type Host, type TunnelStatus } from "./api";

/** The forwards of the host whose tab is in front, with their live state.
 *  Started by hand rather than on connect: a forward binds a local port, and
 *  binding ports behind the user's back is how you collide with whatever else
 *  is already listening. */
export function TunnelBar({ host }: { host: Host }) {
  const rules = (host.tunnels ?? []).filter((t) => t.enabled);
  const [status, setStatus] = useState<Record<string, TunnelStatus>>({});
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (rules.length === 0) return;
    const ids = rules.map((r) => r.id);
    let live = true;
    const poll = () =>
      api
        .tunnelStatus(ids)
        .then((rows) => {
          if (!live) return;
          setStatus(Object.fromEntries(rows.map((r) => [r.ruleId, r])));
        })
        .catch(() => {});
    poll();
    // ssh can exit at any time (a refused port, a dropped network), and the
    // only place that shows is its status.
    const timer = setInterval(poll, 3000);
    return () => {
      live = false;
      clearInterval(timer);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [host.id, rules.length]);

  if (rules.length === 0) return null;

  return (
    <div className="tunnel-bar">
      <span className="muted">Forwards:</span>
      {rules.map((r) => {
        const s = status[r.id];
        const label =
          r.kind === "dynamic"
            ? `SOCKS :${r.listenPort}`
            : `${r.kind === "local" ? "L" : "R"} :${r.listenPort} → ${r.destHost}:${r.destPort}`;
        return (
          <span className="tunnel-chip" key={r.id}>
            <span className={`dot ${s?.running ? "live" : ""}`} />
            <span>{r.name || label}</span>
            {s?.running ? (
              <button onClick={() => api.tunnelStop(r.id).then(() => setStatus((p) => ({ ...p, [r.id]: { ...s, running: false, message: null } })))}>
                Stop
              </button>
            ) : (
              <button
                onClick={() =>
                  api
                    .tunnelStart(host.id, r.id)
                    .then(() => setError(null))
                    .catch((e) => setError(String(e)))
                }
              >
                Start
              </button>
            )}
            {!s?.running && s?.message && (
              <span className="why" title={s.message}>
                {s.message}
              </span>
            )}
          </span>
        );
      })}
      {error && <span className="why">{error}</span>}
    </div>
  );
}
