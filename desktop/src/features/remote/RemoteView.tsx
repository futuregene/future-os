import type { AppSettings } from "../../integrations/storage/appSettings";
import type { RemoteStatus } from "./remoteClient";
import { QRCodeSVG } from "qrcode.react";
import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { ConfirmDeleteDialog } from "../../components/layout/EntityDialogs";
import { LeftPanelTitlebarToggle } from "../../components/layout/LeftPanelTitlebarToggle";
import { Button } from "../../components/ui/Button";
import { openExternalUrl } from "../../integrations/storage/files";
import { cn } from "../../lib/cn";
import { usePolling } from "../../lib/usePolling";
import { startWindowDrag } from "../../lib/windowDrag";
import {
  remoteFailurePresentation,
  startRemote,
  stopRemote,
  unpairRemote,
} from "./remoteClient";

interface RemoteViewProps {
  appSettings: AppSettings;
  leftPanelExpanded: boolean;
  onChangeSettings: (patch: Partial<AppSettings>) => void;
  onToggleLeftPanel: () => void;
  /**
   * Shared remote bridge status polled at the app level — the same source
   * that feeds the sidebar indicator dot, so they always agree.
   */
  remoteStatus: RemoteStatus | null;
  /** Refresh the shared status immediately after a user action. */
  onRefreshRemote: () => Promise<void>;
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

export function RemoteView({
  leftPanelExpanded,
  onToggleLeftPanel,
  remoteStatus,
  onRefreshRemote,
}: RemoteViewProps) {
  const { t } = useTranslation("remote");
  const [copied, setCopied] = useState(false);
  const [busy, setBusy] = useState(false);
  const [confirmOpen, setConfirmOpen] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // --- derived from the shared app-level status (same source as the sidebar indicator) ---
  const phase = remoteStatus?.phase ?? "stopped";
  const running = phase !== "stopped" && phase !== "failed" && phase !== "revoked";
  const connected = phase === "ready";
  const reconnecting = phase === "connecting" || phase === "reconnecting" || phase === "refreshing";
  // Backend `status()` now includes the persisted pair_id even when stopped, so
  // this is authoritative for "paired" across all states: idle, running, and
  // previously-stopped-but-credential-still-here.  Empty when truly unpaired.
  const isPaired = Boolean(remoteStatus?.pairId);
  const terminal = phase === "failed" || phase === "revoked";
  const failure = terminal && remoteStatus?.reason
    ? remoteFailurePresentation(remoteStatus.reason)
    : null;
  const errorText = failure
    ? t(`failure.${failure.messageKey}`, { code: failure.supportCode })
    : error;
  const showError = Boolean(failure || error);

  const pairingCode = remoteStatus?.pairingCode ?? null;
  const [now, setNow] = useState(() => Date.now());
  usePolling(() => setNow(Date.now()), 1000, { enabled: pairingCode != null });
  const remainingSeconds = useMemo(() => {
    const expiresAt = remoteStatus?.pairingCodeExpiresAt;
    if (!pairingCode || expiresAt == null)
      return null;
    return Math.max(0, expiresAt - Math.floor(now / 1000));
  }, [pairingCode, remoteStatus?.pairingCodeExpiresAt, now]);
  const pairingQrValue = useMemo(
    () =>
      pairingCode
        ? `futureos://remote/pair?code=${encodeURIComponent(pairingCode)}&desktopId=${
          encodeURIComponent(remoteStatus?.desktopId ?? "")
        }&desktopKey=${encodeURIComponent(remoteStatus?.desktopPublicKey ?? "")}`
        : null,
    [pairingCode, remoteStatus?.desktopId, remoteStatus?.desktopPublicKey],
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
    <section className="flex h-full min-h-0 flex-col bg-surface">
      <header
        className="flex h-12 shrink-0 select-none items-center justify-between border-b border-line-soft/40 px-4"
        onMouseDown={startWindowDrag}
      >
        <div className="flex min-w-0 flex-1 items-center" data-tauri-drag-region>
          <LeftPanelTitlebarToggle expanded={leftPanelExpanded} onToggle={onToggleLeftPanel} />
          <span className="truncate text-sm font-semibold text-ink">{t("title")}</span>
        </div>
      </header>

      <div className="min-h-0 flex-1 overflow-y-auto p-8">
        <div className="mx-auto w-full max-w-xl space-y-6">
          <p className="text-sm text-ink-muted">
            {t("description")}
            {" "}
            <button
              className="text-accent hover:underline"
              onClick={() => void openExternalUrl("https://future-os.cn/#download-mobile")}
              type="button"
            >
              {t("downloadMobile")}
            </button>
          </p>

          {isPaired && !pairingCode
            ? (
                <div className="rounded-lg border border-line-soft bg-surface-subtle p-4">
                  <div className="flex flex-wrap items-center gap-x-2 gap-y-1">
                    <span
                      className={cn(
                        "inline-block size-2 shrink-0 rounded-full",
                        connected
                          ? "bg-accent"
                          : reconnecting
                            ? "animate-pulse bg-warning"
                            : showError
                              ? "bg-danger"
                              : "bg-ink-muted/60",
                      )}
                    />
                    <span className="min-w-0 truncate text-sm font-medium text-ink">
                      {t(reconnecting ? "reconnectingAs" : connected ? "connectedAs" : "pairedAs", { pairId: formatPairId(remoteStatus?.pairId) })}
                    </span>
                    <div className="ml-auto flex shrink-0 flex-wrap items-center gap-2">
                      {phase !== "failed"
                        ? (
                            <Button
                              disabled={busy}
                              onClick={() => void (running ? handleStop() : handleStart())}
                              size="sm"
                              variant="secondary"
                            >
                              {running ? t("disconnect") : phase === "revoked" ? t("pairAgain") : t("connect")}
                            </Button>
                          )
                        : null}
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
                  {busy
                    ? <p aria-live="polite" className="text-sm text-ink-muted">{t("preparingPairingHint")}</p>
                    : (
                        <Button onClick={() => void handleStart()} variant="primary">
                          {t("pairAndStart")}
                        </Button>
                      )}
                </div>
              )
            : null}

          <p className="text-xs text-ink-muted">{t("note")}</p>
        </div>
      </div>

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
    </section>
  );
}
