import type { AppSettings } from "../../integrations/storage/appSettings";
import type { RemotePairingStatus, RemoteStatus } from "./remoteClient";
import { QRCodeSVG } from "qrcode.react";
import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { LeftPanelTitlebarToggle } from "../../components/layout/LeftPanelTitlebarToggle";
import { Button } from "../../components/ui/Button";
import { cn } from "../../lib/cn";
import { useAsyncResource } from "../../lib/useAsyncResource";
import { usePolling } from "../../lib/usePolling";
import { startWindowDrag } from "../../lib/windowDrag";
import {
  getRemotePairingStatus,
  getRemoteStatus,
  openUrl,
  startRemote,
  stopRemote,
  unpairRemote,
} from "./remoteClient";

interface RemoteViewProps {
  appSettings: AppSettings;
  leftPanelExpanded: boolean;
  onChangeSettings: (patch: Partial<AppSettings>) => void;
  onToggleLeftPanel: () => void;
}

function formatCountdown(totalSeconds: number): string {
  const minutes = Math.floor(totalSeconds / 60);
  const seconds = totalSeconds % 60;
  return `${minutes}:${seconds.toString().padStart(2, "0")}`;
}

export function RemoteView({ leftPanelExpanded, onToggleLeftPanel }: RemoteViewProps) {
  const { t } = useTranslation("remote");
  const [copied, setCopied] = useState(false);
  const { data: loadedStatus } = useAsyncResource<RemoteStatus | null>(getRemoteStatus, [], null);
  const { data: loadedPairing } = useAsyncResource<RemotePairingStatus | null>(
    getRemotePairingStatus,
    [],
    null,
  );
  const [status, setStatus] = useState<RemoteStatus | null>(null);
  const [pairing, setPairing] = useState<RemotePairingStatus | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [errorCode, setErrorCode] = useState<string | null>(null);

  useEffect(() => {
    if (loadedStatus)
      setStatus(loadedStatus);
  }, [loadedStatus]);

  useEffect(() => {
    if (loadedPairing)
      setPairing(loadedPairing);
  }, [loadedPairing]);

  const running = status?.running ?? false;
  const isPaired = pairing?.paired ?? false;
  const activeErrorCode = errorCode ?? (error ? null : status?.errorCode ?? null);
  const errorText = activeErrorCode ? t(`error.${activeErrorCode}`) : error;
  const showError = Boolean(activeErrorCode || error);

  usePolling(async () => {
    try {
      const [nextStatus, nextPairing] = await Promise.all([
        getRemoteStatus(),
        getRemotePairingStatus(),
      ]);
      setStatus(nextStatus);
      setPairing(nextPairing);
    }
    catch {
      // Keep the last known status on a failed poll.
    }
  }, 5000, { enabled: running && !busy });

  const pairingCode = status?.pairingCode ?? null;
  const [now, setNow] = useState(() => Date.now());
  usePolling(() => setNow(Date.now()), 1000, { enabled: pairingCode != null });
  const remainingSeconds = useMemo(() => {
    const expiresAt = status?.pairingCodeExpiresAt;
    if (!pairingCode || expiresAt == null)
      return null;
    return Math.max(0, expiresAt - Math.floor(now / 1000));
  }, [pairingCode, status?.pairingCodeExpiresAt, now]);
  const pairingQrValue = useMemo(
    () =>
      pairingCode
        ? `futureos://remote/pair?code=${encodeURIComponent(pairingCode)}&desktopId=${
          encodeURIComponent(status?.desktopId ?? "")
        }&desktopKey=${encodeURIComponent(status?.desktopPublicKey ?? "")}`
        : null,
    [pairingCode, status?.desktopId, status?.desktopPublicKey],
  );

  async function handleStart() {
    setBusy(true);
    setError(null);
    setErrorCode(null);
    try {
      const next = await startRemote({});
      setStatus(next);
      setPairing(await getRemotePairingStatus());
    }
    catch {
      setErrorCode("generic");
    }
    finally {
      setBusy(false);
    }
  }

  async function handleStop() {
    setBusy(true);
    setError(null);
    setErrorCode(null);
    try {
      setStatus(await stopRemote());
    }
    catch {
      setErrorCode("generic");
    }
    finally {
      setBusy(false);
    }
  }

  async function handleUnpair() {
    setBusy(true);
    setError(null);
    setErrorCode(null);
    try {
      setStatus(await unpairRemote());
      setPairing(await getRemotePairingStatus());
    }
    catch {
      setErrorCode("generic");
    }
    finally {
      setBusy(false);
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
      setErrorCode(null);
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
          <p className="text-sm text-ink-muted">{t("description")}</p>

          <div className="rounded-lg border border-line-soft bg-surface-subtle p-4">
            <div className="flex flex-wrap items-center gap-2">
              <span className={cn("inline-block size-2 rounded-full", running ? "bg-accent" : "bg-ink-muted/60")} />
              <span className="text-sm font-medium text-ink">{running ? t("running") : t("notRunning")}</span>
              {running && status?.webUrl
                ? (
                    <span className="flex items-center gap-1 text-xs text-ink-muted">
                      ·
                      <span>{t("webClient")}</span>
                      <button
                        className="text-accent underline"
                        onClick={() => void openUrl(status.webUrl!)}
                        type="button"
                      >
                        {status.webUrl}
                      </button>
                    </span>
                  )
                : null}
            </div>
          </div>

          {isPaired && !running
            ? (
                <div className="rounded-lg border border-line-soft bg-surface-subtle p-4 text-sm">
                  <div className="flex items-center gap-2">
                    <span className="inline-block size-2 rounded-full bg-accent" />
                    <span className="font-medium text-ink">
                      {t("pairedAs", { pairId: pairing?.pairId ?? "" })}
                    </span>
                  </div>
                </div>
              )
            : null}

          {showError
            ? (
                <div className="rounded-md border border-danger-line bg-danger-soft px-3 py-2 text-sm text-danger">{errorText}</div>
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
                    <Button onClick={() => void copyCode()} size="sm" variant="secondary">
                      {copied ? t("copied") : t("copy")}
                    </Button>
                  </div>
                  <div className="grid gap-4 md:grid-cols-[auto_1fr] md:items-center">
                    {pairingQrValue
                      ? (
                          <div className="mx-auto rounded-lg border border-line bg-surface p-3 md:mx-0">
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
                    <div className="min-w-0 space-y-2">
                      <p className="text-xs font-medium text-ink-soft">{t("pairingQrLabel")}</p>
                      <code className="block break-all rounded bg-surface px-3 py-2 text-xs text-ink">{pairingQrValue}</code>
                      <p className="text-xs text-ink-muted">{t("pairingCodeHint")}</p>
                    </div>
                  </div>
                </div>
              )
            : null}

          <div className="flex flex-wrap gap-2">
            {running
              ? (
                  <>
                    <Button disabled={busy} onClick={() => void handleStop()} variant="secondary">
                      {t("stop")}
                    </Button>
                    {isPaired
                      ? (
                          <Button disabled={busy} onClick={() => void handleUnpair()} variant="secondary">
                            {t("unpair")}
                          </Button>
                        )
                      : null}
                  </>
                )
              : (
                  <Button disabled={busy} onClick={() => void handleStart()} variant="primary">
                    {t("pairAndStart")}
                  </Button>
                )}
          </div>

          <p className="text-xs text-ink-muted">{t("note")}</p>
        </div>
      </div>
    </section>
  );
}
