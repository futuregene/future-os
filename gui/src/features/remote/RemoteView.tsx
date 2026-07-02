import type { AppSettings } from "../../integrations/storage/appSettings";
import type { RemoteStatus } from "./remoteClient";
import { useEffect, useState } from "react";
import { Trans, useTranslation } from "react-i18next";
import { cn } from "../../lib/cn";
import { getRemoteStatus, startRemote, stopRemote } from "./remoteClient";

interface RemoteViewProps {
  appSettings: AppSettings;
  onChangeSettings: (patch: Partial<AppSettings>) => void;
}

export function RemoteView({ appSettings, onChangeSettings }: RemoteViewProps) {
  const { t } = useTranslation("remote");
  const [natsUrl, setNatsUrl] = useState(appSettings.remoteNatsUrl);
  const [pairId, setPairId] = useState(appSettings.remotePairId);
  const [status, setStatus] = useState<RemoteStatus | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    void getRemoteStatus()
      .then((next) => {
        if (!cancelled)
          setStatus(next);
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, []);

  const running = status?.running ?? false;

  async function handleStart() {
    setBusy(true);
    setError(null);
    onChangeSettings({ remoteNatsUrl: natsUrl, remotePairId: pairId, remoteEnabled: true });
    try {
      setStatus(await startRemote({ natsUrl, pairId }));
    }
    catch (caught) {
      setError(caught instanceof Error ? caught.message : String(caught));
    }
    finally {
      setBusy(false);
    }
  }

  async function handleStop() {
    setBusy(true);
    setError(null);
    onChangeSettings({ remoteEnabled: false });
    try {
      setStatus(await stopRemote());
    }
    catch (caught) {
      setError(caught instanceof Error ? caught.message : String(caught));
    }
    finally {
      setBusy(false);
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
            {running
              ? (
                  <span className="text-xs text-ink-muted">
                    ·
                    {" "}
                    {status?.natsUrl}
                    {" "}
                    · pair
                    {" "}
                    {status?.pairId}
                  </span>
                )
              : null}
          </div>
        </div>

        <div className="space-y-4">
          <label className="block space-y-1">
            <span className="text-sm font-medium text-ink-soft">{t("natsUrlLabel")}</span>
            <input
              className="w-full rounded-md border border-line-soft bg-surface px-3 py-2 text-sm text-ink outline-none focus:border-accent"
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
            <span className="text-sm font-medium text-ink-soft">{t("pairIdLabel")}</span>
            <input
              className="w-full rounded-md border border-line-soft bg-surface px-3 py-2 text-sm text-ink outline-none focus:border-accent"
              disabled={running || busy}
              onChange={event => setPairId(event.target.value)}
              placeholder="DEVPAIR"
              value={pairId}
            />
          </label>
        </div>

        {error
          ? (
              <div className="rounded-md border border-danger/40 bg-danger-soft px-3 py-2 text-sm text-danger">{error}</div>
            )
          : null}

        <div className="flex gap-2">
          {running
            ? (
                <button
                  className="rounded-md border border-line-soft bg-surface px-4 py-2 text-sm font-medium text-ink-soft transition-colors hover:bg-surface-subtle disabled:opacity-50"
                  disabled={busy}
                  onClick={() => void handleStop()}
                  type="button"
                >
                  {t("stop")}
                </button>
              )
            : (
                <button
                  className="rounded-md border border-accent bg-accent-soft px-4 py-2 text-sm font-medium text-accent transition-colors hover:opacity-90 disabled:opacity-50"
                  disabled={busy || !natsUrl.trim() || !pairId.trim()}
                  onClick={() => void handleStart()}
                  type="button"
                >
                  {t("start")}
                </button>
              )}
        </div>

        <p className="text-xs text-ink-muted">
          {t("note")}
        </p>
      </div>
    </section>
  );
}
