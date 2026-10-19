import type { TFunction } from "i18next";
import type { RemoteDevice, RemoteStatus } from "./remoteClient";
import { QRCodeSVG } from "qrcode.react";
import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { ConfirmDeleteDialog } from "../../components/layout/EntityDialogs";
import { Badge } from "../../components/ui/Badge";
import { Button } from "../../components/ui/Button";
import { Switch } from "../../components/ui/Switch";
import { openExternalUrl } from "../../integrations/storage/files";
import { emitFutureEvent } from "../../lib/futureEvents";
import { usePolling } from "../../lib/usePolling";
import {
  addRemoteDevice,
  remoteConnectionPresentation,
  startRemote,
  stopRemote,
  takeoverRemote,
  unpairRemote,
  unpairRemoteDevice,
} from "./remoteClient";

/**
 * "Let other devices connect here" — the host half of the Remote page.
 *
 * A section rather than a page: the page shell (title bar, scroll container,
 * width) belongs to the hub that shows this beside the other direction, and its
 * heading comes from there too.
 *
 * The section is **one switch plus what the switch enables**. Everything under
 * the switch is configuration *for* a running connection — the paired clients,
 * the invitation that adds another one, the auto-connect preference — so none of
 * it is shown while the switch is off. An "off" section that still lists clients
 * and preferences reads as a service that is up, and the start/stop buttons it
 * used to carry asked the reader to work out which of several controls happened
 * to be the real on/off.
 */
export interface RemoteHostSectionProps {
  /**
   * Shared remote bridge status polled at the app level — the same source
   * that feeds the sidebar indicator dot, so they always agree.
   */
  remoteStatus: RemoteStatus | null;
  /** Refresh the shared status immediately after a user action. */
  onRefreshRemote: () => Promise<void>;
  /** Whether this machine reconnects its paired devices when the app opens. */
  autoConnect: boolean;
  onToggleAutoConnect: (value: boolean) => void;
}

function formatCountdown(totalSeconds: number): string {
  const minutes = Math.floor(totalSeconds / 60);
  const seconds = totalSeconds % 60;
  return `${minutes}:${seconds.toString().padStart(2, "0")}`;
}

/**
 * Short display form of a pair id: drop the `pair_` prefix (redundant next to the
 * "Paired as" label) and upper-case the remainder so it reads as a code, not a
 * lowercase hash. Unknown shapes pass through unchanged.
 */
function formatPairId(pairId: string | null | undefined): string {
  if (!pairId)
    return "";
  return (pairId.startsWith("pair_") ? pairId.slice(5) : pairId).toUpperCase();
}

/**
 * The platform's cap is a count over rows it owns, so the only honest place to
 * learn that a device cannot be added is the refusal itself.
 */
function isLimitError(err: unknown): boolean {
  const text = err instanceof Error ? err.message : String(err);
  return text.includes("pairing_limit_reached");
}

/** What to call a device: what it said it was, else what we can see. */
function deviceLabel(
  device: Pick<RemoteDevice, "client" | "connected">,
  t: TFunction<"remote">,
): string {
  if (device.client) {
    return t(`clientKind.${device.client.kind}`, {
      defaultValue: t("clientKind.other"),
      name: device.client.name,
    });
  }
  return device.connected ? t("clientUnknown") : t("devicePaired");
}

export function RemoteHostSection({
  autoConnect,
  onRefreshRemote,
  onToggleAutoConnect,
  remoteStatus,
}: RemoteHostSectionProps) {
  const { t } = useTranslation("remote");
  const { t: tHub } = useTranslation("remoteHub");
  const [copied, setCopied] = useState(false);
  const [busy, setBusy] = useState(false);
  const [confirmOpen, setConfirmOpen] = useState(false);
  /** The device whose unpair is being confirmed, if any. */
  const [confirmUnpair, setConfirmUnpair] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

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
  // Backend `status()` now includes the persisted pair_id even when stopped, so
  // this is authoritative for "paired" across all states: idle, running, and
  // previously-stopped-but-credential-still-here.  Empty when truly unpaired.
  const isPaired = Boolean(remoteStatus?.pairId);
  const terminal = phase === "failed" || phase === "revoked";
  const failure = terminal && presentation?.messageKey && presentation.messageKey !== "connection" && presentation.messageKey !== "devicePreparing"
    ? { messageKey: presentation.messageKey, supportCode: presentation.supportCode }
    : null;
  const errorText = failure
    ? t(`failure.${failure.messageKey}`, { code: failure.supportCode })
    : error;
  const showError = Boolean(failure || error);

  // Every paired device, when the backend knows about them. An older backend
  // sends none, and the single-device layout below is what that machine uses.
  const devices = useMemo(() => remoteStatus?.devices ?? [], [remoteStatus?.devices]);
  const multi = devices.length > 0;
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
  const confirmUnpairName = useMemo(() => {
    const device = devices.find(item => item.pairId === confirmUnpair);
    return device ? deviceLabel(device, t) : formatPairId(confirmUnpair);
  }, [confirmUnpair, devices, t]);
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

  async function handleUnpair() {
    setBusy(true);
    setError(null);
    try {
      await unpairRemote();
    }
    catch (err) {
      console.error("remote unpair failed:", err);
      setError(t("error.generic"));
    }
    finally {
      setBusy(false);
      await onRefreshRemote();
    }
  }

  async function handleAddDevice() {
    setBusy(true);
    setError(null);
    try {
      await addRemoteDevice();
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

  async function handleTakeover(pairId: string) {
    setBusy(true);
    setError(null);
    try {
      await takeoverRemote(pairId);
    }
    catch (err) {
      console.error("remote takeover failed:", err);
      setError(t("error.generic"));
    }
    finally {
      setBusy(false);
      await onRefreshRemote();
    }
  }

  async function handleUnpairDevice(pairId: string) {
    setBusy(true);
    setError(null);
    try {
      await unpairRemoteDevice(pairId);
    }
    catch (err) {
      console.error("remote unpair device failed:", err);
      setError(t("error.generic"));
    }
    finally {
      setBusy(false);
      await onRefreshRemote();
    }
  }

  async function copyCode() {
    if (!pairingQrValue)
      return;
    try {
      await navigator.clipboard.writeText(pairingQrValue);
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

        {/* Everything below configures a running bridge, so it exists only
            while the switch is on. */}
        {running
          ? (
              <>
                {multi && isPaired
                  ? (
                      // The bridge's own state, above the device list. The list
                      // says which device is driving; this says whether the
                      // machine can be reached at all — and, when it cannot, why
                      // and what to do. A list of devices alone read as
                      // "everything is fine" during an outage, because every row
                      // is simply standing by.
                      <div className="flex flex-wrap items-center gap-x-2 gap-y-1 rounded-lg border border-line-soft bg-surface-subtle px-4 py-2">
                        <Badge tone={connected ? "accent" : connecting ? "warning" : "danger"}>
                          {t(connectionStatusKey)}
                        </Badge>
                        <span className="min-w-0 flex-1 truncate text-xs text-ink-muted">
                          {connected ? "" : t("connectionHint")}
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

                {multi
                  ? (
                      <div className="rounded-lg border border-line-soft bg-surface-subtle">
                        <ul className="divide-y divide-line-soft">
                          {devices.map(device => (
                            <li
                              className="flex flex-wrap items-center gap-x-2 gap-y-1 px-4 py-2"
                              key={device.pairId}
                            >
                              <Badge
                                tone={device.control === "active"
                                  ? "accent"
                                  : device.pairingCode != null
                                    ? "info"
                                    : device.connected
                                      ? "warning"
                                      : "neutral"}
                              >
                                {device.control === "active"
                                  ? t("deviceActive")
                                  : device.pairingCode != null
                                    ? t("deviceWaiting")
                                    : device.connected
                                      ? t("deviceStandby")
                                      : t("deviceOffline")}
                              </Badge>
                              <span className="min-w-0 truncate text-sm text-ink">{deviceLabel(device, t)}</span>
                              <span className="min-w-0 truncate text-xs text-ink-muted">{formatPairId(device.pairId)}</span>
                              <div className="ml-auto flex shrink-0 items-center gap-2">
                                {/* A standby device can be given the machine from here, which
                                    is the desktop-side twin of the device's own "take over".
                                    A device that is not connected cannot: it has no bridge to
                                    command, so handing it the machine would leave the device
                                    that *is* connected standing by for nobody. */}
                                {device.control === "standby" && device.connected && device.pairingCode == null
                                  ? (
                                      <Button
                                        disabled={busy}
                                        onClick={() => void handleTakeover(device.pairId)}
                                        size="sm"
                                        variant="secondary"
                                      >
                                        {t("takeover")}
                                      </Button>
                                    )
                                  : null}
                                <Button
                                  disabled={busy}
                                  onClick={() => setConfirmUnpair(device.pairId)}
                                  size="sm"
                                  variant="secondary"
                                >
                                  {t("unpair")}
                                </Button>
                              </div>
                            </li>
                          ))}
                        </ul>
                        <div className="flex flex-wrap items-center gap-x-3 gap-y-2 border-t border-line-soft px-4 py-2">
                          <span className="min-w-0 flex-1 text-xs text-ink-muted">{t("addDeviceHint")}</span>
                          <Button
                            disabled={busy}
                            onClick={() => void handleAddDevice()}
                            size="sm"
                            variant="secondary"
                          >
                            {t("addDevice")}
                          </Button>
                        </div>
                      </div>
                    )
                  : null}

                {!multi && isPaired && !pairingCode
                  ? (
                      <div className="rounded-lg border border-line-soft bg-surface-subtle p-4">
                        <div className="flex flex-wrap items-center gap-x-2 gap-y-1">
                          <Badge tone={connected ? "accent" : connecting ? "warning" : "danger"}>
                            {t(connectionStatusKey)}
                          </Badge>
                          {/* What is on the other end, when it said so. The wording is
                              device-neutral without it: the entry used to be labelled
                              "phone", which stopped being true once a computer could
                              connect here too. */}
                          {connected
                            ? (
                                <span className="min-w-0 truncate text-sm text-ink">
                                  {remoteStatus?.client
                                    ? t(`clientKind.${remoteStatus.client.kind}`, {
                                        defaultValue: t("clientKind.other"),
                                        name: remoteStatus.client.name,
                                      })
                                    : t("clientUnknown")}
                                </span>
                              )
                            : null}
                          <span className="min-w-0 truncate text-sm text-ink-muted">{formatPairId(remoteStatus?.pairId)}</span>
                          <div className="ml-auto flex shrink-0 flex-wrap items-center gap-2">
                            <Button
                              disabled={busy}
                              onClick={() => setConfirmOpen(true)}
                              size="sm"
                              variant="secondary"
                            >
                              {t("unpair")}
                            </Button>
                          </div>
                        </div>
                      </div>
                    )
                  : null}

                {pairingCode
                  ? (
                      <div className="space-y-2 rounded-lg border border-line-soft bg-surface-subtle p-4">
                        <div className="flex items-center justify-between">
                          <span className="flex items-center gap-2 text-sm font-medium text-ink-soft">
                            {t("pairingCodeLabel")}
                            {remainingSeconds != null && (
                              <span className="text-xs font-normal text-ink-muted">
                                {t("pairingCodeExpiresIn", { time: formatCountdown(remainingSeconds) })}
                              </span>
                            )}
                          </span>
                          <div className="flex items-center gap-2">
                            <Button onClick={() => void copyCode()} size="sm" variant="secondary">
                              {copied ? t("copied") : t("copy")}
                            </Button>
                            <Button disabled={busy} onClick={() => void handleStop()} size="sm" variant="secondary">
                              {t("cancel")}
                            </Button>
                          </div>
                        </div>
                        <div className="flex flex-col items-center gap-2">
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
                          <p className="text-center text-xs font-medium text-ink-soft">{t("pairingQrLabel")}</p>
                          <p className="text-center text-xs text-ink-muted">{t("pairingCodeHint")}</p>
                        </div>
                      </div>
                    )
                  : null}

                {!isPaired && !pairingCode
                  ? (
                      <div className="flex h-9 items-center">
                        <p aria-live="polite" className="text-sm text-ink-muted">{t("preparingPairingHint")}</p>
                      </div>
                    )
                  : null}

                <div className="flex items-center justify-between gap-3 border-t border-line-soft pt-3">
                  <div className="min-w-0">
                    <div className="text-sm text-ink">{tHub("autoConnectTitle")}</div>
                    <div className="mt-0.5 text-xs text-ink-muted">{tHub("autoConnectDescription")}</div>
                  </div>
                  <Switch
                    checked={autoConnect}
                    label={tHub("autoConnectTitle")}
                    onChange={onToggleAutoConnect}
                  />
                </div>

                <p className="text-xs text-ink-muted">
                  {t("note")}
                  {" "}
                  <button
                    className="text-accent hover:underline"
                    onClick={() => void openExternalUrl("https://future-os.cn/#download-mobile")}
                    type="button"
                  >
                    {t("downloadMobile")}
                  </button>
                </p>
              </>
            )
          : null}
      </div>

      <ConfirmDeleteDialog
        description={t("unpairDeviceConfirmDesc", { name: confirmUnpairName })}
        error={null}
        onClose={() => setConfirmUnpair(null)}
        onConfirm={() => {
          const pairId = confirmUnpair;
          setConfirmUnpair(null);
          if (pairId)
            void handleUnpairDevice(pairId);
        }}
        open={confirmUnpair != null}
        submitting={false}
        title={t("unpairDeviceConfirmTitle")}
      >
        <p className="text-sm text-ink-soft">{formatPairId(confirmUnpair)}</p>
      </ConfirmDeleteDialog>
      <ConfirmDeleteDialog
        description={t("unpairConfirmDesc")}
        error={null}
        onClose={() => setConfirmOpen(false)}
        onConfirm={() => {
          setConfirmOpen(false);
          void handleUnpair();
        }}
        open={confirmOpen}
        submitting={false}
        title={t("unpairConfirmTitle")}
      >
        <p className="text-sm text-ink-soft">{t("pairedAs", { pairId: formatPairId(remoteStatus?.pairId) })}</p>
      </ConfirmDeleteDialog>
    </>
  );
}
