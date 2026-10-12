import type { RemotePeer } from "./remotePeerClient";
import { emit } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "../../components/ui/Button";
import { Dialog } from "../../components/ui/Dialog";
import { errorMessage } from "../../lib/errors";
import { startWindowDrag } from "../../lib/windowDrag";
import { listRemotePeers, setRemotePeerLabel, unpairRemotePeer } from "./remotePeerClient";
import { RemotePeerSettings } from "./RemotePeerSettings";
import { RemoteTasksView } from "./RemoteTasksView";

/**
 * The Tauri event a settings window raises when a task run names a conversation
 * it cannot show itself.
 *
 * Cross-window, so the app's own event bus (`futureEvents`) will not do: that one
 * is DOM `CustomEvent`s on `window`, which by construction reach only the window
 * that fired them.
 */
export const OPEN_REMOTE_SESSION_EVENT = "remote-peer-open-session";

/**
 * One machine's settings, in a window of its own (see
 * `remote_peer_open_settings_window`).
 *
 * It is a separate window because those settings belong to the *other* machine:
 * beside this machine's own state, "auto-upgrade skills" stops meaning one
 * thing. The main window stays usable while this is open, which is the point —
 * the user can go back to their conversations without losing the page.
 *
 * The window is identified by the `desktop` query parameter, and reads the peer
 * itself rather than being handed it: the main window may rename or disconnect
 * that machine while this window is open, and a value passed once would be the
 * stale one.
 */
export function PeerSettingsWindow({ desktopId }: { desktopId: string }) {
  const { t } = useTranslation("remotePeer");
  const [peer, setPeer] = useState<RemotePeer | null>(null);
  const [missing, setMissing] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [unpairing, setUnpairing] = useState(false);
  const [warning, setWarning] = useState<string | null>(null);
  const [tasksOpen, setTasksOpen] = useState(false);

  const refresh = useCallback(async () => {
    try {
      const peers = await listRemotePeers();
      const found = peers.find(item => item.desktopId === desktopId) ?? null;
      setPeer(found);
      // Unpairing does not reload this window — it *is* about that machine, and
      // there is nothing left to show. Saying so beats an empty settings page.
      setMissing(found === null);
      setError(null);
    }
    catch (err) {
      setError(errorMessage(err));
    }
  }, [desktopId]);

  useEffect(() => {
    void refresh();
    // Polled because the *other* side of this page changes without telling us:
    // the link drops, or the main window renames the machine. Same cadence as
    // the main window's peer list, which this page is a view of.
    const timer = setInterval(() => void refresh(), 5_000);
    return () => clearInterval(timer);
  }, [refresh]);

  function close() {
    void getCurrentWindow().close().catch(() => {});
  }

  async function unpair() {
    setUnpairing(true);
    setError(null);
    try {
      const pending = await unpairRemotePeer(desktopId);
      await refresh();
      setUnpairing(false);
      if (pending) {
        // The pairing is gone either way, but a revoke the platform never
        // received is something the user has to be told. It is shown here, where
        // the button was pressed, rather than raised as a toast on the main
        // window — this page is already about that machine, and it is the one
        // still on screen.
        setWarning(t("unpairPendingWarning"));
        return;
      }
      close();
    }
    catch (err) {
      setError(errorMessage(err));
      setUnpairing(false);
    }
  }

  return (
    <section className="flex h-full min-h-0 flex-col bg-surface">
      <header
        className="flex h-12 shrink-0 select-none items-center justify-between gap-2 border-b border-line-soft/40 px-4"
        onMouseDown={startWindowDrag}
      >
        <span className="min-w-0 flex-1 truncate text-sm font-semibold text-ink">
          {t("settingsWindowTitle")}
        </span>
        <Button onClick={close} size="sm" variant="ghost">{t("close")}</Button>
      </header>

      <div className="min-h-0 flex-1 overflow-y-auto p-6">
        {missing
          ? (
              <div className="mx-auto w-full max-w-md space-y-3 text-center">
                <p className="text-sm text-ink">{t("settingsWindowMissing")}</p>
                {warning ? <p className="text-xs text-warning">{warning}</p> : null}
                <Button onClick={close} variant="secondary">{t("close")}</Button>
              </div>
            )
          : peer === null
            ? <p className="text-center text-xs text-ink-muted">{t("loadingPeers")}</p>
            : tasksOpen
              ? (
                  <RemoteTasksView
                    leftPanelExpanded
                    onBack={() => setTasksOpen(false)}
                    onOpenSession={(_, sessionId) => {
                      // The conversation is shown by the *main* window: this one
                      // has no conversation pane, and rendering the transcript
                      // here too would be a second copy of the same conversation
                      // with its own composer and its own approvals. The main
                      // window raises itself when it handles this.
                      void emit(OPEN_REMOTE_SESSION_EVENT, { desktopId, sessionId });
                    }}
                    onToggleLeftPanel={() => {}}
                    peer={peer}
                  />
                )
              : (
                  <RemotePeerSettings
                    onBack={close}
                    onChanged={() => void refresh()}
                    onOpenTasks={() => setTasksOpen(true)}
                    onUnpair={() => setUnpairing(true)}
                    onUpdateLabel={patch =>
                      void setRemotePeerLabel(desktopId, patch).then(refresh).catch(err => setError(errorMessage(err)))}
                    peer={peer}
                  />
                )}
      </div>

      {/* Unpairing removes the pairing, so it is confirmed here rather than on
          the main window: this is where the button was pressed. */}
      {unpairing && peer
        ? (
            <Dialog
              description={t("unpairConfirmDesc", { name: peer.name ?? desktopId })}
              footer={(
                <>
                  <Button onClick={() => setUnpairing(false)} variant="ghost">{t("cancel")}</Button>
                  <Button onClick={() => void unpair()} variant="danger">{t("unpair")}</Button>
                </>
              )}
              onClose={() => setUnpairing(false)}
              open
              title={t("unpairConfirmTitle")}
            >
              <span />
            </Dialog>
          )
        : null}

      {error ? <p className="px-4 pb-2 text-xs text-danger">{error}</p> : null}
    </section>
  );
}
