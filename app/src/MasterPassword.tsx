import { useEffect, useState } from "react";
import { api, type VaultStatus } from "./api";

const MIN_LEN = 8;

/** Mirrors core::lock::check_master — characters, not bytes, so a Bengali
 *  password is held to the same rule as an English one. */
function tooShort(p: string): boolean {
  return [...p].length < MIN_LEN;
}

const FORGET_WARNING =
  "If you forget it, your saved passwords cannot be recovered — not by you, and not by BHTerminal. Your hosts, folders and snippets are not affected.";

/** Shown INSTEAD of the app while saved passwords are locked: nothing behind
 *  it is usable until it is unlocked, so nothing behind it is drawn. */
export function UnlockScreen({ onUnlocked }: { onUnlocked: () => void }) {
  const [password, setPassword] = useState("");
  const [remember, setRemember] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [forgot, setForgot] = useState(false);
  const [movedTo, setMovedTo] = useState<string | null>(null);

  async function unlock() {
    if (!password || busy) return;
    setBusy(true);
    setError(null);
    try {
      await api.vaultUnlock(password, remember);
      onUnlocked();
    } catch (e) {
      setError(String(e));
      setPassword("");
    }
    setBusy(false);
  }

  async function reset() {
    setBusy(true);
    setError(null);
    try {
      setMovedTo(await api.vaultResetForgotten());
    } catch (e) {
      setError(String(e));
    }
    setBusy(false);
  }

  return (
    <div className="lock-screen">
      <div className="modal lock-card">
        <h2>BHTerminal is locked</h2>
        {movedTo ? (
          <>
            <p className="done-line">
              Saved passwords were reset. Your hosts, folders and snippets are all still here — you
              will be asked for each server's password the next time you connect.
            </p>
            <p className="done-line">
              The locked passwords were not destroyed. They were moved to <code>{movedTo}</code>, in
              case the master password comes back to you.
            </p>
            <div className="modal-actions">
              <button className="primary" onClick={onUnlocked}>
                Continue
              </button>
            </div>
          </>
        ) : forgot ? (
          <>
            <p className="done-line">
              Without the master password, saved server passwords cannot be decrypted. Resetting
              starts an empty password store; your hosts, folders and snippets stay as they are.
            </p>
            {error && <div className="error">{error}</div>}
            <div className="modal-actions">
              <button onClick={() => setForgot(false)} disabled={busy}>
                Back
              </button>
              <button className="danger" onClick={reset} disabled={busy}>
                {busy ? "Resetting…" : "Reset saved passwords"}
              </button>
            </div>
          </>
        ) : (
          <>
            <label className="field">
              <span>Master password</span>
              <input
                type="password"
                autoComplete="current-password"
                value={password}
                autoFocus
                disabled={busy}
                onChange={(e) => setPassword(e.target.value)}
                onKeyDown={(e) => e.key === "Enter" && unlock()}
              />
            </label>
            <label className="check">
              <input
                type="checkbox"
                checked={remember}
                onChange={(e) => setRemember(e.target.checked)}
              />
              <span>Remember on this computer</span>
            </label>
            {error && <div className="error">{error}</div>}
            <div className="modal-actions spread">
              <button className="link" onClick={() => setForgot(true)} disabled={busy}>
                Forgot it?
              </button>
              <button className="primary" onClick={unlock} disabled={busy || !password}>
                {busy ? "Unlocking…" : "Unlock"}
              </button>
            </div>
          </>
        )}
      </div>
    </div>
  );
}

interface SheetProps {
  /** True for the one-time question asked on first start. */
  intro?: boolean;
  onClose: () => void;
}

/** Setting, changing and removing the master password, in one sheet. */
export function MasterPasswordSheet({ intro, onClose }: SheetProps) {
  const [status, setStatus] = useState<VaultStatus | null>(null);
  const [asking, setAsking] = useState(!!intro);
  const [current, setCurrent] = useState("");
  const [password, setPassword] = useState("");
  const [again, setAgain] = useState("");
  const [remember, setRemember] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [done, setDone] = useState<string | null>(null);
  const [confirmRemove, setConfirmRemove] = useState(false);

  const refresh = () =>
    api.vaultStatus().then(setStatus).catch((e) => setError(String(e)));
  useEffect(() => {
    refresh();
  }, []);

  async function run(action: () => Promise<void>, message: string) {
    if (busy) return;
    setBusy(true);
    setError(null);
    try {
      await action();
      setDone(message);
      setCurrent("");
      setPassword("");
      setAgain("");
      await refresh();
    } catch (e) {
      setError(String(e));
    }
    setBusy(false);
  }

  function checkNew(): boolean {
    if (tooShort(password)) {
      setError(`Use a master password of at least ${MIN_LEN} characters.`);
      return false;
    }
    if (password !== again) {
      setError("The two passwords do not match.");
      return false;
    }
    return true;
  }

  async function notNow() {
    await api.vaultSkipSetup().catch(() => {});
    onClose();
  }

  if (!status) {
    // A failed status check must be SEEN: returning nothing here made the
    // menu item look dead and silently skipped the first-run question.
    if (!error) return null;
    return (
      <div className="modal-backdrop">
        <div className="modal">
          <h2>Master password</h2>
          <div className="error">{error}</div>
          <div className="modal-actions">
            <button className="primary" onClick={onClose}>
              Close
            </button>
          </div>
        </div>
      </div>
    );
  }

  return (
    <div className="modal-backdrop" onClick={intro || busy ? undefined : onClose}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        {asking ? (
          <>
            <h2>Protect saved passwords with a master password?</h2>
            <p className="done-line">
              BHTerminal can lock your saved server passwords behind one master password. Without one
              they are still stored encrypted, and open by themselves when the app starts.
            </p>
            <p className="done-line">You can set, change or remove it later: ＋ at the top of Sessions → Master password….</p>
            <div className="modal-actions">
              <button onClick={notNow}>Not now</button>
              <button className="primary" autoFocus onClick={() => setAsking(false)}>
                Set a master password
              </button>
            </div>
          </>
        ) : confirmRemove ? (
          <>
            <h2>Remove the master password?</h2>
            <p className="done-line">
              Saved passwords stay encrypted on this computer, but BHTerminal will open them by itself
              again — nobody will be asked for a password.
            </p>
            {error && <div className="error">{error}</div>}
            <div className="modal-actions">
              <button onClick={() => setConfirmRemove(false)} disabled={busy}>
                Back
              </button>
              <button
                className="danger"
                disabled={busy}
                onClick={() =>
                  run(
                    () => api.vaultRemoveMaster(current),
                    "Master password removed. Saved passwords stay encrypted and open by themselves again."
                  ).then(() => setConfirmRemove(false))
                }
              >
                {busy ? "Removing…" : "Remove master password"}
              </button>
            </div>
          </>
        ) : done ? (
          <>
            <h2>Master password</h2>
            <p className="done-line">{done}</p>
            <div className="modal-actions">
              <button className="primary" onClick={onClose}>
                Done
              </button>
            </div>
          </>
        ) : !status.hasMaster ? (
          <>
            <h2>Set a master password</h2>
            <label className="field">
              <span>Master password</span>
              <input
                type="password"
                autoComplete="new-password"
                value={password}
                autoFocus
                onChange={(e) => setPassword(e.target.value)}
              />
            </label>
            <label className="field">
              <span>Again</span>
              <input type="password" autoComplete="new-password" value={again} onChange={(e) => setAgain(e.target.value)} />
            </label>
            <label className="check">
              <input
                type="checkbox"
                checked={remember}
                onChange={(e) => setRemember(e.target.checked)}
              />
              <span>Remember on this computer</span>
            </label>
            <p className="hint">
              {remember
                ? "BHTerminal will not ask for it on this computer. It is still needed to change it, and to export saved passwords."
                : "BHTerminal will ask for it each time it starts. Closing the window keeps it running — quit from the menu bar icon to lock it."}
            </p>
            <div className="warnings">
              <p>{FORGET_WARNING}</p>
            </div>
            {error && <div className="error">{error}</div>}
            <div className="modal-actions">
              <button onClick={intro ? notNow : onClose} disabled={busy}>
                {intro ? "Not now" : "Cancel"}
              </button>
              <button
                className="primary"
                disabled={busy || !password}
                onClick={() =>
                  checkNew() &&
                  run(
                    () => api.vaultSetMaster(null, password, remember),
                    remember
                      ? "Master password set. This computer will remember it."
                      : "Master password set. BHTerminal will ask for it the next time it starts."
                  )
                }
              >
                {busy ? "Saving…" : "Set master password"}
              </button>
            </div>
          </>
        ) : (
          <>
            <h2>Master password</h2>
            <label className="field">
              <span>Current master password</span>
              <input
                type="password"
                autoComplete="current-password"
                value={current}
                autoFocus
                onChange={(e) => setCurrent(e.target.value)}
              />
              <small>
                Needed for every change below.
                {status.remembered &&
                  " Forgotten it? Saved passwords keep working here because this computer remembers it, but changing or removing it needs it."}
              </small>
            </label>

            <label className="check">
              <input
                type="checkbox"
                checked={status.remembered}
                disabled={busy}
                onChange={(e) => {
                  if (!current) {
                    setError("Enter the current master password first.");
                    return;
                  }
                  const on = e.target.checked;
                  run(
                    () => api.vaultSetRemember(current, on),
                    on
                      ? "This computer will remember the master password."
                      : "This computer will ask for the master password each time BHTerminal starts."
                  );
                }}
              />
              <span>Remember on this computer</span>
            </label>

            <div className="sheet-section">
              <label className="field">
                <span>New master password</span>
                <input type="password" autoComplete="new-password" value={password} onChange={(e) => setPassword(e.target.value)} />
              </label>
              <label className="field">
                <span>Again</span>
                <input type="password" autoComplete="new-password" value={again} onChange={(e) => setAgain(e.target.value)} />
              </label>
            </div>

            {error && <div className="error">{error}</div>}
            <div className="modal-actions spread">
              <button
                className="danger"
                disabled={busy || !current}
                onClick={() => setConfirmRemove(true)}
              >
                Remove
              </button>
              <span className="grow" />
              <button onClick={onClose} disabled={busy}>
                Cancel
              </button>
              <button
                className="primary"
                disabled={busy || !current || !password}
                onClick={() =>
                  checkNew() &&
                  run(
                    () => api.vaultSetMaster(current, password, status.remembered),
                    "Master password changed. Backups exported before keep their old password."
                  )
                }
              >
                {busy ? "Saving…" : "Change"}
              </button>
            </div>
          </>
        )}
      </div>
    </div>
  );
}
