import type { RemoteStatus } from "./remoteClient";
import { ChevronRight } from "lucide-react";
import { QRCodeSVG } from "qrcode.react";
import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { Badge } from "../../components/ui/Badge";
import { Button } from "../../components/ui/Button";
import { Dialog } from "../../components/ui/Dialog";
import { Switch } from "../../components/ui/Switch";
import { emitFutureEvent } from "../../lib/futureEvents";
import { usePolling } from "../../lib/usePolling";
import {
  addRemoteDevice,
  remoteConnectionPresentation,
  startRemote,
  stopRemote,
} from "./remoteClient";
import {
  deviceLabel,
  deviceState,
  formatCountdown,
  formatPairId,
  isLimitError,
} from "./remoteHostFormat";

/**
 * "Let other devices connect here" — the host half of the Remote page.
 *
 * The section is **one switch plus the list it enables**. The switch is the
 * only start/stop control; while it is off nothing below it is shown (no list,
 * no add button, no preference), because an "off" section that still lists
 * clients reads as a service that is up.
 *
 * While it is on, the section is a **list only**: one row per paired client,
 * each a door to that client's own page (its identity, disconnect, delete) —
 * the same "list here, details after clicking in" shape the other direction of
 * the page uses. The one exception is adding a client: it opens a floating
 * window that carries the QR code *and* the pairing code, so the invitation is
 * visible without navigating away from the list.
 */
export interface RemoteHostSectionProps {
  /**
   * Shared remote bridge status polled at the app level — the same source
   * that feeds the sidebar indicator dot, so they always agree.
   */
  remoteStatus: RemoteStatus | null;
  /** Refresh the shared status immediately after a user action. */
  onRefreshRemote: () => Promise<void>;
  /** Open one paired client's page. */
  onOpenDevice: (pairId: string) => void;
  /** Open this direction's settings (the auto-connect preference). */
  onOpenSettings: () => void;
}

export function RemoteHostSection({
  onOpenDevice,
  onOpenSettings,
  onRefreshRemote,
  remoteStatus,
}: RemoteHostSectionProps) {
  const { t } = useTranslation("remote");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  /** The floating add-client window (QR + pairing code). */
  const [addOpen, setAddOpen] = useState(false);
  const [copied, setCopied] = useState(false);

  // --- derived from the shared app-level status (same source as the sidebar indicator) ---
  const phase = remoteStatus?.phase ?? "stopped";
  const running = phase !== "stopped" && phase !== "failed" && phase !== "revoked";
  const presentation = remoteConnectionPresentation(remoteStatus);
  const connected = presentation?.level === "connected";
  const connecting = presentation?.level === "connecting";
  const connectionStatusKey = connected
    ? "statusConnected"
    : connecting
      ? "statusConnecting"
      : "statusDisconnected";
  const terminal = phase === "failed" || phase === "revoked";
  const failure = terminal && presentation?.messageKey && presentation.messageKey !== "connection" && presentation.messageKey !== "devicePreparing"
    ? { messageKey: presentation.messageKey, supportCode: presentation.supportCode }
    : null;
  const errorText = failure
    ? t(`failure.${failure.messageKey}`, { code: failure.supportCode })
    : error;
  const showError = Boolean(failure || error);

  const devices = useMemo(() => {
    const listed = remoteStatus?.devices ?? [];
    if (listed.length > 0)
      return listed;
    // An older backend sends no per-device rows; the top-level fields still
    // describe the one paired device, so project them into a row instead of
    // showing an empty list for a machine that is in fact paired.
    if (!remoteStatus?.pairId)
      return [];
    return [{
      pairId: remoteStatus.pairId,
      phase: remoteStatus.phase,
      control: "active" as const,
      connected: remoteStatus.phase === "ready",
      client: remoteStatus.client,
      pairingCode: remoteStatus.pairingCode,
      pairingCodeExpiresAt: remoteStatus.pairingCodeExpiresAt,
      desktopId: remoteStatus.desktopId,
    }];
  }, [remoteStatus]);
  // The invitation belongs to whichever device has not been claimed yet; the
  // top-level field is the controller's projection and is only a fallback.
  const pendingDevice = devices.find(device => device.pairingCode != null) ?? null;
  const pairingCode = pendingDevice?.pairingCode ?? remoteStatus?.pairingCode ?? null;
  const [now, setNow] = useState(() => Date.now());
  usePolling(() => setNow(Date.now()), 1000, { enabled: pairingCode != null });
  const remainingSeconds = useMemo(() => {
    const expiresAt = pendingDevice
      ? pendingDevice.pairingCodeExpiresAt
      : remoteStatus?.pairingCodeExpiresAt;
    if (!pairingCode || expiresAt == null)
      return null;
    return Math.max(0, expiresAt - Math.floor(now / 1000));
  }, [pairingCode, pendingDevice, remoteStatus?.pairingCodeExpiresAt, now]);
  const pairingQrValue = useMemo(
    () =>
      pairingCode?.startsWith("futureos://remote/pair?") ? pairingCode : null,
    [pairingCode],
  );

  async function handleStart() {
    setBusy(true);
    setError(null);
    try {
      await startRemote({});
    }
    catch (err) {
      // Keep internal detail in the console; the user gets a stable action.
      console.error("remote start failed:", err);
      setError(t("error.generic"));
    }
    finally {
      await onRefreshRemote();
      // Keep the preparation message visible until the refreshed status has
      // supplied the pairing code, avoiding a one-frame return of the button.
      setBusy(false);
    }
  }

  async function handleStop() {
    setBusy(true);
    setError(null);
    try {
      await stopRemote();
    }
    catch (err) {
      console.error("remote stop failed:", err);
      setError(t("error.generic"));
    }
    finally {
      setBusy(false);
      setAddOpen(false);
      await onRefreshRemote();
    }
  }

  /**
   * The switch is the only start/stop control. Turning it on when the account
   * needs attention sends the user to the step that can actually succeed,
   * rather than starting a bridge that the platform will refuse.
   */
  function handleToggle(next: boolean) {
    if (!next) {
      void handleStop();
      return;
    }
    if (presentation?.action === "loginAgain") {
      emitFutureEvent("show-onboarding", undefined);
      return;
    }
    void handleStart();
  }

  async function handleAddDevice() {
    setBusy(true);
    setError(null);
    try {
      await addRemoteDevice();
      // The invitation exists from here on: the window shows it as soon as the
      // refreshed status carries the code.
      setAddOpen(true);
    }
    catch (err) {
      console.error("remote add device failed:", err);
      setError(isLimitError(err) ? t("error.limitReached") : t("error.generic"));
    }
    finally {
      setBusy(false);
      await onRefreshRemote();
    }
  }

  async function copyCode() {
    if (!pairingCode)
      return;
    try {
      await navigator.clipboard.writeText(pairingCode);
      setCopied(true);
      setTimeout(setCopied, 1500, false);
    }
    catch {
      setError(t("copyFailed"));
    }
  }

  return (
    <>
      <div className="space-y-4">
        <div className="flex items-center justify-between gap-3">
          <div className="min-w-0">
            <div className="text-sm text-ink">{t("hostToggleTitle")}</div>
            <div className="mt-0.5 text-xs text-ink-muted">{t("hostToggleDescription")}</div>
          </div>
          <Switch
            checked={running}
            disabled={busy}
            label={t("hostToggleTitle")}
            onChange={handleToggle}
          />
        </div>

        {/* Feedback about the switch itself — why it could not come on, or why
            the bridge failed — stays visible while it is off. It is not a
            setting, and hiding it left an off switch with no explanation. */}
        {showError
          ? (
              <div className="flex items-center gap-3 rounded-md border border-danger-line bg-danger-soft px-3 py-2 text-sm text-danger">
                <span className="min-w-0 flex-1">{errorText}</span>
              </div>
            )
          : null}

        {remoteStatus?.warningCode === "web_bind"
          ? (
              <div className="rounded-md border border-warning/30 bg-warning/10 px-3 py-2 text-sm text-ink-soft">
                {t("warning.webBind", { code: "LC002" })}
              </div>
            )
          : null}

        {/* Turning the switch on is asynchronous: the bridge has to start and
            report back. Say so while that happens — otherwise the switch looks
            inert for the whole start. */}
        {!running && busy
          ? <p aria-live="polite" className="text-sm text-ink-muted">{t("preparingPairingHint")}</p>
          : null}

        {/* Everything below is the list the switch enables. */}
        {running
          ? (
              <>
                {!connected && connecting
                  ? (
                      <div className="flex flex-wrap items-center gap-x-2 gap-y-1 rounded-lg border border-line-soft bg-surface-subtle px-4 py-2">
                        <Badge tone={connecting ? "warning" : "danger"}>
                          {t(connectionStatusKey)}
                        </Badge>
                        <span className="min-w-0 flex-1 truncate text-xs text-ink-muted">
                          {t("connectionHint")}
                        </span>
                        {presentation?.action === "loginAgain"
                          ? (
                              <Button
                                disabled={busy}
                                onClick={() => emitFutureEvent("show-onboarding", undefined)}
                                size="sm"
                                variant="secondary"
                              >
                                {t("loginAgain")}
                              </Button>
                            )
                          : null}
                      </div>
                    )
                  : null}

                <div className="flex items-center justify-between gap-3">
                  <span className="text-sm font-medium text-ink">{t("clientsTitle")}</span>
                  <div className="flex items-center gap-1">
                    <Button onClick={onOpenSettings} size="sm" variant="ghost">
                      {t("hostSettings")}
                    </Button>
                    <Button disabled={busy} onClick={() => void handleAddDevice()} size="sm" variant="secondary">
                      {t("addClient")}
                    </Button>
                  </div>
                </div>

                <div className="rounded-lg border border-line-soft bg-surface-subtle">
                  {devices.length === 0
                    ? (
                        <p className="px-4 py-3 text-sm text-ink-muted">{t("noClients")}</p>
                      )
                    : (
                        <ul className="divide-y divide-line-soft">
                          {devices.map((device) => {
                            const state = deviceState(device, t);
                            return (
                              <li key={device.pairId}>
                                {/* A row is a door, not a control panel: the list
                                    says who is paired and what state they are
                                    in; disconnect/delete live on that client's
                                    own page. */}
                                <button
                                  className="flex w-full items-center gap-2 px-4 py-2 text-left hover:bg-surface"
                                  onClick={() => onOpenDevice(device.pairId)}
                                  type="button"
                                >
                                  <span className="min-w-0 flex-1 truncate text-sm text-ink">
                                    {deviceLabel(device, t)}
                                  </span>
                                  <span className="shrink-0 text-xs text-ink-muted">{formatPairId(device.pairId)}</span>
                                  <Badge tone={state.tone}>{state.label}</Badge>
                                  <ChevronRight className="size-4 shrink-0 text-ink-muted" />
                                </button>
                              </li>
                            );
                          })}
                        </ul>
                      )}
                </div>
              </>
            )
          : null}
      </div>

      {/* The add-client window: QR code *and* pairing code, side by side, so the
          invitation can be shown without leaving the list. */}
      <Dialog
        description={t("addClientDescription")}
        footer={(
          <Button onClick={() => setAddOpen(false)} variant="secondary">
            {t("done")}
          </Button>
        )}
        onClose={() => setAddOpen(false)}
        open={addOpen}
        title={t("addClientTitle")}
      >
        <div className="flex flex-col items-center gap-3">
          {pairingQrValue
            ? (
                <div className="rounded-lg border border-line bg-surface p-3">
                  <QRCodeSVG
                    aria-label={t("pairingQrLabel")}
                    bgColor="transparent"
                    className="text-ink-strong"
                    fgColor="currentColor"
                    level="M"
                    role="img"
                    size={176}
                    value={pairingQrValue}
                  />
                </div>
              )
            : null}
          <p className="text-center text-xs text-ink-muted">{t("pairingQrLabel")}</p>
          {pairingCode
            ? (
                <div className="w-full space-y-1">
                  <div className="flex items-center justify-between gap-2">
                    <span className="text-xs font-medium text-ink-soft">
                      {t("pairingCodeLabel")}
                      {remainingSeconds != null && (
                        <span className="ml-2 font-normal text-ink-muted">
                          {t("pairingCodeExpiresIn", { time: formatCountdown(remainingSeconds) })}
                        </span>
                      )}
                    </span>
                    <Button onClick={() => void copyCode()} size="sm" variant="secondary">
                      {copied ? t("copied") : t("copy")}
                    </Button>
                  </div>
                  <code
                    className="block w-full break-all rounded-md border border-line-soft bg-surface-subtle px-3 py-2 font-mono text-xs text-ink"
                    data-testid="pairing-code"
                  >
                    {pairingCode}
                  </code>
                </div>
              )
            : (
                <p aria-live="polite" className="text-sm text-ink-muted">{t("preparingPairingHint")}</p>
              )}
          <p className="text-center text-xs text-ink-muted">{t("pairingCodeHint")}</p>
        </div>
      </Dialog>
    </>
  );
}
