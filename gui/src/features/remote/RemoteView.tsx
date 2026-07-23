import type { AppSettings } from "../../integrations/storage/appSettings";
import type { RemotePairingStatus, RemoteStatus } from "./remoteClient";
import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "../../components/ui/Button";
import { cn } from "../../lib/cn";
import { errorMessage } from "../../lib/errors";
import { useAsyncResource } from "../../lib/useAsyncResource";
import { usePolling } from "../../lib/usePolling";
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
  onChangeSettings: (patch: Partial<AppSettings>) => void;
}

function formatCountdown(totalSeconds: number): string {
  const minutes = Math.floor(totalSeconds / 60);
  const seconds = totalSeconds % 60;
  return `${minutes}:${seconds.toString().padStart(2, "0")}`;
}

export function RemoteView(_: RemoteViewProps) {
  const { t } = useTranslation("remote");
  const [copied, setCopied] = useState(false);
  // Mirror the loaded status locally so start/stop can apply their returned
  // status without waiting for a refetch. A failed initial fetch is non-fatal
  // (status stays null → "not running") so its error isn't surfaced here.
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

  // While running, poll so a dropped connection (or a stop from elsewhere) is
  // reflected instead of staying stuck on "running". Best-effort: a failed poll
  // keeps the last known status rather than flashing an error (see CLAUDE.md §4).
  usePolling(async () => {
    try {
      setStatus(await getRemoteStatus());
    }
    catch {
      // Keep the last known status on a failed poll.
    }
  }, 5000, { enabled: running && !busy });

  // The pairing code lives in the backend status (returned while fresh), so it
  // survives navigating away and back — no longer a show-once value. The
  // backend stops returning it once it expires; this 1s tick only drives the
  // countdown display in between polls.
  const pairingCode = status?.pairingCode ?? null;
  // 1s tick drives the countdown display between the 5s status polls; `now`
  // is the tick value, used in the countdown computation below.
  const [now, setNow] = useState(() => Date.now());
  usePolling(() => setNow(Date.now()), 1000, { enabled: pairingCode != null });
  const remainingSeconds = useMemo(() => {
    const expiresAt = status?.pairingCodeExpiresAt;
    if (!pairingCode || expiresAt == null)
      return null;
    return Math.max(0, expiresAt - Math.floor(now / 1000));
  }, [pairingCode, status?.pairingCodeExpiresAt, now]);

  async function handleStart() {
    setBusy(true);
    setError(null);
    try {
      const next = await startRemote({});
      setStatus(next);
      setPairing(await getRemotePairingStatus());
    }
    catch (caught) {
      setError(errorMessage(caught));
    }
    finally {
      setBusy(false);
    }
  }

  async function handleStop() {
    setBusy(true);
    setError(null);
    try {
      setStatus(await stopRemote());
    }
    catch (caught) {
      setError(errorMessage(caught));
    }
    finally {
      setBusy(false);
    }
  }

  async function handleUnpair() {
    setBusy(true);
    setError(null);
    try {
      setStatus(await unpairRemote());
      setPairing(await getRemotePairingStatus());
    }
    catch (caught) {
      setError(errorMessage(caught));
    }
    finally {
      setBusy(false);
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
    <section className="flex h-full min-h-0 flex-col overflow-y-auto bg-surface p-8">
      <div className="mx-auto w-full max-w-xl space-y-6">
        <header>
          <h1 className="text-lg font-semibold text-ink">{t("title")}</h1>
          <p className="mt-1 text-sm text-ink-muted">
            {t("description")}
          </p>
        </header>

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
          {/* LAN URL for a phone on the same network (localhost only works on
              this machine). Selectable so it can be copied by hand. */}
          {running && status?.webLanUrl
            ? (
                <p className="mt-2 select-all text-xs text-ink-muted">
                  {t("webClientLan")}
                  {" "}
                  <span className="text-ink-soft">{status.webLanUrl}</span>
                </p>
              )
            : null}
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

        {error
          ? (
              <div className="rounded-md border border-danger-line bg-danger-soft px-3 py-2 text-sm text-danger">{error}</div>
            )
          : null}

        {/* A bridge that stopped on its own (e.g. pairing revoked by the web
            client) explains itself here instead of a bare "not running". */}
        {status?.error
          ? (
              <div className="rounded-md border border-danger-line bg-danger-soft px-3 py-2 text-sm text-danger">{status.error}</div>
            )
          : null}

        {pairingCode
          ? (
              <div className="space-y-2 rounded-lg border border-line-soft bg-surface-subtle p-4">
                <div className="flex items-center justify-between">
                  <span className="flex items-center gap-2 text-sm font-medium text-ink-soft">
                    {t("pairingCodeLabel")}
                    {remainingSeconds != null && (
                      <span className="font-normal text-xs text-ink-muted">
                        {t("pairingCodeExpiresIn", { time: formatCountdown(remainingSeconds) })}
                      </span>
                    )}
                  </span>
                  <Button onClick={() => void copyCode()} size="sm" variant="secondary">
                    {copied ? t("copied") : t("copy")}
                  </Button>
                </div>
                <code className="block break-all rounded bg-surface px-3 py-2 text-xs text-ink">{pairingCode}</code>
                <p className="text-xs text-ink-muted">{t("pairingCodeHint")}</p>
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
                <Button
                  disabled={busy}
                  onClick={() => void handleStart()}
                  variant="primary"
                >
                  {t("pairAndStart")}
                </Button>
              )}
        </div>

        <p className="text-xs text-ink-muted">
          {t("note")}
        </p>
      </div>
    </section>
  );
}
