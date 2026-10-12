import { useCallback, useEffect, useState } from "react";
import { api, type Available } from "./api";

/** GitHub allows 60 unauthenticated calls an hour per IP, and this runs on
 *  every launch, so an automatic check is throttled. A check the user asked
 *  for always runs. */
const THROTTLE_MS = 30 * 60 * 1000;
const LAST_CHECK_KEY = "bhterminal.lastUpdateCheck";

export interface UpdateState {
  available: Available | null;
  /** Only set for a check the user asked for: an automatic one stays silent. */
  error: string | null;
  checking: boolean;
  /** True once a manual check has come back with nothing newer. */
  upToDate: boolean;
}

export function useUpdateCheck(currentVersionSink?: (v: string) => void) {
  const [state, setState] = useState<UpdateState>({
    available: null,
    error: null,
    checking: false,
    upToDate: false,
  });

  const check = useCallback(
    async (manual: boolean) => {
      if (!manual) {
        const last = Number(localStorage.getItem(LAST_CHECK_KEY) ?? 0);
        if (Date.now() - last < THROTTLE_MS) return;
      }
      setState((s) => ({ ...s, checking: true, error: null, upToDate: false }));
      try {
        const query = await api.updateQuery();
        currentVersionSink?.(query.currentVersion);
        const res = await fetch(query.url, {
          headers: { Accept: "application/vnd.github+json" },
        });
        // Record the attempt even when it failed, so a rate-limited or
        // offline launch does not retry on every window focus.
        localStorage.setItem(LAST_CHECK_KEY, String(Date.now()));
        if (!res.ok) {
          throw new Error(
            res.status === 403
              ? "GitHub rate-limited the check — try again in a few minutes."
              : `Couldn't reach GitHub (HTTP ${res.status}).`
          );
        }
        const found = await api.updateEvaluate(await res.text());
        setState({ available: found, error: null, checking: false, upToDate: !found });
      } catch (e) {
        // An automatic check fails silently; only a requested one reports.
        setState((s) => ({
          ...s,
          checking: false,
          error: manual ? String(e instanceof Error ? e.message : e) : null,
        }));
      }
    },
    [currentVersionSink]
  );

  useEffect(() => {
    check(false);
  }, [check]);

  return {
    ...state,
    checkNow: () => check(true),
    dismiss: () => setState((s) => ({ ...s, available: null })),
  };
}
