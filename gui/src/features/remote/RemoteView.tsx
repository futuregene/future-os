import type { AppSettings } from "../../integrations/storage/appSettings";
import type { RemotePairingStatus, RemoteStatus } from "./remoteClient";
import { useEffect, useState } from "react";
import { Trans, useTranslation } from "react-i18next";
import { Button } from "../../components/ui/Button";
import { TextInput } from "../../components/ui/TextInput";
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

export function RemoteView({ appSettings, onChangeSettings }: RemoteViewProps) {
  const { t } = useTranslation("remote");
  const [natsUrl, setNatsUrl] = useState(appSettings.remoteNatsUrl);
  const [pairId, setPairId] = useState(appSettings.remotePairId);
  const [token, setToken] = useState("");
  const [pairingCode, setPairingCode] = useState<string | null>(null);
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

  // Seed the form once settings load — the useState initializers ran before the
  // async settings arrived. Keyed on the persisted values, so it won't clobber
  // in-progress edits (typing changes local state, not appSettings, and this
  // component is the only writer of these fields).
  useEffect(() => {
    setNatsUrl(appSettings.remoteNatsUrl);
    setPairId(appSettings.remotePairId);
  }, [appSettings.remoteNatsUrl, appSettings.remotePairId]);

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

  async function handleStart() {
    setBusy(true);
    setError(null);
    setPairingCode(null);
    try {
      const next = await startRemote({
        natsUrl,
        pairId: pairId.trim() || undefined,
        accessToken: token.trim(),
      });
      setStatus(next);
      if (next.pairingCode)
        setPairingCode(next.pairingCode);
      // Persist only after a successful start, so a failed attempt doesn't leave
      // `remoteEnabled` true.
      onChangeSettings({ remoteNatsUrl: natsUrl, remotePairId: pairId, remoteEnabled: true });
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
      onChangeSettings({ remoteEnabled: false });
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
      setPairingCode(null);
      setPairing(await getRemotePairingStatus());
      onChangeSettings({ remoteEnabled: false });
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
        </div>

        <div className="space-y-4">
          <label className="block space-y-1">
            <span className="text-sm font-medium text-ink-soft">{t("natsUrlLabel")}</span>
            <TextInput
              disabled={running || busy}
              onChange={event => setNatsUrl(event.target.value)}
              placeholder="nats://localhost:4222"
              value={natsUrl}
            />
            <span className="block text-xs text-ink-muted">
              <Trans i18nKey="natsUrlHint" ns="remote" components={[<code key="url" />]} />
            </span>
          </label>

          <label className="block space-y-1">
            <span className="text-sm font-medium text-ink-soft">{t("tokenLabel")}</span>
            <TextInput
              disabled={running || busy}
              onChange={event => setToken(event.target.value)}
              placeholder="devpairingtoken"
              type="password"
              value={token}
            />
            <span className="block text-xs text-ink-muted">{t("tokenHint")}</span>
          </label>

          <label className="block space-y-1">
            <span className="text-sm font-medium text-ink-soft">{t("pairIdLabel")}</span>
            <TextInput
              disabled={running || busy}
              onChange={event => setPairId(event.target.value)}
              placeholder="DEVPAIR"
              value={pairId}
            />
            <span className="block text-xs text-ink-muted">{t("pairIdHint")}</span>
          </label>
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

        {pairingCode
          ? (
              <div className="space-y-2 rounded-lg border border-line-soft bg-surface-subtle p-4">
                <div className="flex items-center justify-between">
                  <span className="text-sm font-medium text-ink-soft">{t("pairingCodeLabel")}</span>
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
                  disabled={busy || !natsUrl.trim() || !token.trim()}
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
