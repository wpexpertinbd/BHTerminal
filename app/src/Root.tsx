import { useEffect, useState } from "react";
import App from "./App";
import { api, type VaultStatus } from "./api";
import { MasterPasswordSheet, UnlockScreen } from "./MasterPassword";

/** Decides what the window shows before the app proper: the unlock screen
 *  while saved passwords are locked, and the one-time master-password
 *  question on top of the app the first time it starts. */
export default function Root() {
  const [status, setStatus] = useState<VaultStatus | null>(null);
  const [error, setError] = useState<string | null>(null);

  const refresh = () =>
    api
      .vaultStatus()
      .then(setStatus)
      .catch((e) => setError(String(e)));

  useEffect(() => {
    refresh();
  }, []);

  if (error) {
    return (
      <div className="lock-screen">
        <div className="modal lock-card">
          <h2>BHTerminal couldn't open saved passwords</h2>
          <div className="error">{error}</div>
          <div className="modal-actions">
            <button
              className="primary"
              onClick={() => {
                setError(null);
                refresh();
              }}
            >
              Try again
            </button>
          </div>
        </div>
      </div>
    );
  }
  if (!status) return null;
  if (status.locked) return <UnlockScreen onUnlocked={refresh} />;

  return (
    <>
      <App />
      {status.askSetup && (
        <MasterPasswordSheet intro onClose={() => setStatus({ ...status, askSetup: false })} />
      )}
    </>
  );
}
