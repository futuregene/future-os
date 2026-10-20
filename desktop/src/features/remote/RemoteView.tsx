import type { RemoteStatus } from "./remoteClient";
import { ChevronRight } from "lucide-react";
import { QRCodeSVG } from "qrcode.react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { ConfirmDeleteDialog } from "../../components/layout/EntityDialogs";
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
  unpairRemoteDevice,
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
 * The section is **one switch plus the connections it carries**. The switch is
 * the only start/stop control; while it is off nothing below it is shown (no
 * list, no add button, no preference), because an "off" section that still lists
 * connections reads as a service that is up.
 *
 * Turning the switch on is not turning a connection on. Every connection is
 * created one at a time, by "add a device", and a machine with none is simply
 * on and empty. So the two are siblings: the switch decides whether this
 * machine serves its connections at all, and the list is what it serves.
 *
 * While it is on, the section is a **list plus one action**: each row is a door
 * to that connection's own page, the device that is driving this machine can be
 * unpaired from its row (it is the one the machine is serving), and "add a
 * device" sits below the list. Adding opens a floating window that carries the
 * QR code *and* the pairing code, so the invitation is visible without
 * navigating away — and closing it throws the invitation away rather than
 * leaving a half-made connection behind.
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
  /** The floating add-device window (QR + pairing code). */
  const [addOpen, setAddOpen] = useState(false);
  /** The connection this window created, while it is still unclaimed. */
  const [pendingPairId, setPendingPairId] = useState<string | null>(null);
  /** Whether the window has already shown the invitation it was opened for. */
  const invitationShownRef = useRef(false);
  /**
   * When the invitation on screen lapses, captured while it is still shown.
   *
   * The status stops reporting a one-shot code both when a device claims it and
   * when it expires, so "no code" alone cannot say which happened — and the two
   * need different words for the user.
   */
  const invitationExpiryRef = useRef<number | null>(null);
  /** The device whose unpair is being confirmed, if any. */
  const [confirmUnpair, setConfirmUnpair] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);

  // --- derived from the shared app-level status (same source as the sidebar indicator) ---
  const phase = remoteStatus?.phase ?? "stopped";
  const running = phase !== "stopped" && phase !== "failed" && phase !== "revoked";
  const presentation = remoteConnectionPresentation(remoteStatus);
  const terminal = phase === "failed" || phase === "revoked";
  const failure = terminal && presentation?.messageKey && presentation.messageKey !== "connection" && presentation.messageKey !== "devicePreparing"
    ? { messageKey: presentation.messageKey, supportCode: presentation.supportCode }
    : null;
  const errorText = failure
    ? t(`failure.${failure.messageKey}`, { code: failure.supportCode })
    : error;
  const showError = Boolean(failure || error);

  /** Every connection this machine knows about, invited-but-unclaimed included. */
  const allDevices = useMemo(() => {
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
  // An invitation nobody has scanned is not a connection yet: it belongs to the
  // window that is showing it, not to the list. A list that shows it anyway
  // reads as "a device is waiting", which is exactly what the user did not ask
  // for when they turned the switch on.
  const devices = useMemo(
    () => allDevices.filter(device => device.pairingCode == null),
    [allDevices],
  );
  // The one whose invitation this window is showing: the device it created, if
  // it is still unclaimed, else whichever invitation is live.
  const pendingDevice = useMemo(() => {
    const unclaimed = allDevices.filter(device => device.pairingCode != null);
    return unclaimed.find(device => device.pairId === pendingPairId)
      ?? unclaimed[0]
      ?? null;
  }, [allDevices, pendingPairId]);
  const pairingCode = pendingDevice?.pairingCode ?? null;
  const [now, setNow] = useState(() => Date.now());
  usePolling(() => setNow(Date.now()), 1000, { enabled: pairingCode != null });
  const remainingSeconds = useMemo(() => {
    const expiresAt = pendingDevice?.pairingCodeExpiresAt;
    if (!pairingCode || expiresAt == null)
      return null;
    return Math.max(0, expiresAt - Math.floor(now / 1000));
  }, [pairingCode, pendingDevice?.pairingCodeExpiresAt, now]);
  const pairingQrValue = useMemo(
    () =>
      pairingCode?.startsWith("futureos://remote/pair?") ? pairingCode : null,
    [pairingCode],
  );
  const confirmUnpairDevice = useMemo(
    () => devices.find(device => device.pairId === confirmUnpair) ?? null,
    [confirmUnpair, devices],
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
      setPendingPairId(null);
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
      const status = await addRemoteDevice();
      // The returned status is *about* the connection just created; that is the
      // one the window speaks for, even if the list later shows others.
      setPendingPairId(status.pairId || null);
      invitationShownRef.current = false;
      invitationExpiryRef.current = null;
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

  /**
   * Close the add window and drop the connection it was offering.
   *
   * Closing is a decision about *this* connection only: the remote desktop
   * stays on, and the connections that already exist keep running. An
   * invitation that was never scanned has no device behind it, so discarding it
   * leaves nothing half-made in the list.
   */
  const closeAdd = useCallback(async () => {
    invitationShownRef.current = false;
    invitationExpiryRef.current = null;
    setAddOpen(false);
    const discarded = pendingPairId;
    setPendingPairId(null);
    if (!discarded)
      return;
    // Only while it is still unclaimed: a device that scanned the code in the
    // meantime is a real connection, and closing the window is not "unpair it".
    const unclaimed = allDevices.some(
      device => device.pairId === discarded && device.pairingCode != null,
    );
    if (!unclaimed)
      return;
    setBusy(true);
    try {
      await unpairRemoteDevice(discarded);
    }
    catch (err) {
      console.error("remote discard invitation failed:", err);
      setError(t("error.generic"));
    }
    finally {
      setBusy(false);
      await onRefreshRemote();
    }
  }, [allDevices, onRefreshRemote, pendingPairId, t]);

  // The invitation is over once its code leaves the status, and the window has
  // nothing left to show: it carries the invitation, not the connection. A
  // device that claimed it now has a row of its own in the list; an invitation
  // nobody claimed leaves a lapsed code, which only this line can explain.
  // Left open either way, the window falls through to the "preparing" line —
  // the wording for a code that is still being minted — and promises a QR that
  // is never coming.
  useEffect(() => {
    if (pairingCode != null) {
      invitationShownRef.current = true;
      invitationExpiryRef.current ??= pendingDevice?.pairingCodeExpiresAt ?? null;
      return;
    }
    if (!addOpen || !invitationShownRef.current)
      return;
    const lapsed = invitationExpiryRef.current;
    if (lapsed != null && Date.now() >= lapsed * 1000)
      setError(t("pairingCodeExpired"));
    void closeAdd();
  }, [addOpen, closeAdd, pairingCode, pendingDevice?.pairingCodeExpiresAt, t]);

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
            setting, and hiding it left an off switch with no explanation. The
            sign-in action rides here because a refused start is exactly what
            this banner describes. */}
        {showError
          ? (
              <div className="flex items-center gap-3 rounded-md border border-danger-line bg-danger-soft px-3 py-2 text-sm text-danger">
                <span className="min-w-0 flex-1">{errorText}</span>
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

        {/* Everything below is what the switch serves. */}
        {running
          ? (
              <>
                <div className="flex items-center justify-between gap-3">
                  <span className="text-sm font-medium text-ink">{t("clientsTitle")}</span>
                  <Button onClick={onOpenSettings} size="sm" variant="ghost">
                    {t("hostSettings")}
                  </Button>
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
                              <li
                                className="flex items-center gap-2 pr-2"
                                key={device.pairId}
                              >
                                {/* A row is a door, not a control panel: the list
                                    says which connections exist and what state
                                    they are in; disconnect/delete live on that
                                    client's own page. The one action that rides
                                    in the row is "unpair" for the device that is
                                    driving this machine — that is the connection
                                    this machine is actually serving. */}
                                <button
                                  className="flex min-w-0 flex-1 items-center gap-2 px-4 py-2 text-left hover:bg-surface"
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
                                {device.control === "active"
                                  ? (
                                      <Button
                                        disabled={busy}
                                        onClick={() => setConfirmUnpair(device.pairId)}
                                        size="sm"
                                        variant="secondary"
                                      >
                                        {t("unpair")}
                                      </Button>
                                    )
                                  : null}
                              </li>
                            );
                          })}
                        </ul>
                      )}
                </div>

                {/* Adding a connection is its own step below the list, because
                    turning the switch on does not create one. */}
                <div className="flex items-center justify-between gap-3">
                  <span className="min-w-0 flex-1 text-xs leading-5 text-ink-muted">
                    {t("addDeviceHint")}
                  </span>
                  <Button
                    disabled={busy}
                    onClick={() => void handleAddDevice()}
                    size="sm"
                    variant="secondary"
                  >
                    {t("addDevice")}
                  </Button>
                </div>
              </>
            )
          : null}
      </div>

      {/* The add-device window: QR code *and* pairing code, side by side, so the
          invitation can be shown without leaving the list. Closing it cancels
          that one connection — never the remote desktop. */}
      <Dialog
        description={t("addDeviceDescription")}
        footer={(
          <Button onClick={() => void closeAdd()} variant="secondary">
            {t("cancel")}
          </Button>
        )}
        onClose={() => void closeAdd()}
        open={addOpen}
        title={t("addDeviceTitle")}
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

      {/* Unpairing the device that drives this machine, from its row. The
          confirm lives here rather than on the row so an accidental click in a
          list cannot cut off the phone the user is holding. */}
      <ConfirmDeleteDialog
        description={t("unpairDeviceConfirmDesc", {
          name: confirmUnpairDevice ? deviceLabel(confirmUnpairDevice, t) : "",
        })}
        error={null}
        onClose={() => setConfirmUnpair(null)}
        onConfirm={() => {
          const pairId = confirmUnpair;
          setConfirmUnpair(null);
          if (pairId)
            void handleUnpairDevice(pairId);
        }}
        open={confirmUnpair != null}
        submitting={busy}
        title={t("unpairDeviceConfirmTitle")}
      >
        <p className="text-sm text-ink-soft">
          {formatPairId(confirmUnpairDevice?.pairId ?? confirmUnpair)}
        </p>
      </ConfirmDeleteDialog>
    </>
  );
}
