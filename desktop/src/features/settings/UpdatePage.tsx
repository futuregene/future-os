import type { UpdateStatus } from "../../components/layout/hooks/useUpdateChecker";
import type { MaintenanceStatus } from "../../integrations/tauri/useMaintenanceStatus";
import { listen } from "@tauri-apps/api/event";
import { Download, RefreshCw, RotateCcw } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "../../components/ui/Button";
import { openExternalUrl } from "../../integrations/storage/files";
import { invokeCommand } from "../../integrations/tauri/invoke";
import { useBuildInfo } from "../../integrations/tauri/useBuildInfo";
import { useMaintenanceStatus } from "../../integrations/tauri/useMaintenanceStatus";
import { errorMessage } from "../../lib/errors";
import { SettingsSection } from "./SettingsPrimitives";

interface DownloadProgress {
  downloaded: number;
  total: number;
}

/**
 * Software update page backed by Tauri's signed in-place updater.
 * Accepts an optional `cachedStatus` from the background update checker so
 * the result is displayed immediately without a redundant network round-trip.
 */
export function UpdatePage({ cachedStatus }: { cachedStatus?: UpdateStatus | null }) {
  const { t } = useTranslation("settings");
  const build = useBuildInfo();
  const [status, setStatus] = useState<UpdateStatus | null>(cachedStatus ?? null);
  const [checking, setChecking] = useState(false);
  const [checkError, setCheckError] = useState<string | null>(null);
  const [downloading, setDownloading] = useState(false);
  const [progress, setProgress] = useState(0);
  const [installed, setInstalled] = useState(false);
  const [installError, setInstallError] = useState<string | null>(null);

  const { maintenance, setMaintenance, maintenanceError, setMaintenanceError } = useMaintenanceStatus();
  const [downloadError, setDownloadError] = useState<string | null>(null);

  const recovery = maintenance?.update;
  const awaitingRestart = installed || recovery?.outcome === "pending-validation";
  const updateStage = recovery?.stage ?? "downloading";
  const preparing = ["verified", "preparing", "prepared"].includes(updateStage);
  const installing = ["cleanup", "awaiting-exit", "installing"].includes(updateStage);
  const needsRecovery = Boolean(installError && recovery?.outcome && ["failed-restored", "repair-required", "cleanup-failed"].includes(recovery.outcome));
  const needsManualRepair = Boolean(recovery && !recovery.repairAcknowledged && (recovery.outcome === "repair-required" || (recovery.outcome === "interrupted" && ["installing", "awaiting-exit"].includes(recovery.stage))));

  // Tear down the streamed-progress listener if the page unmounts mid-download
  // (the download command itself still settles in the background).
  const mountedRef = useRef(true);
  const unlistenRef = useRef<(() => void) | null>(null);
  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
      unlistenRef.current?.();
      unlistenRef.current = null;
    };
  }, []);

  async function handleCheck() {
    setChecking(true);
    setCheckError(null);
    setInstalled(false);
    setInstallError(null);
    setDownloadError(null);
    try {
      setStatus(await invokeCommand<UpdateStatus>("check_app_update"));
    }
    catch (error) {
      setStatus(null);
      setCheckError(errorMessage(error));
    }
    finally {
      setChecking(false);
    }
  }

  async function handleInstall() {
    if (!status?.hasUpdate)
      return;
    setDownloading(true);
    setProgress(0);
    setInstallError(null);
    try {
      // Subscribe before installing; subscription failures must also release
      // the downloading state rather than becoming unhandled rejections.
      const unlisten = await listen<DownloadProgress>("app-update-progress", (event) => {
        const { downloaded, total } = event.payload;
        if (total > 0 && mountedRef.current)
          setProgress(Math.min(100, Math.round((downloaded / total) * 100)));
      });
      if (!mountedRef.current) {
        unlisten();
        return;
      }
      unlistenRef.current = unlisten;
      await invokeCommand("install_app_update");
      if (mountedRef.current) {
        setProgress(100);
        const result = await invokeCommand<MaintenanceStatus>("app_maintenance_status");
        if (mountedRef.current) {
          setMaintenance(result ?? null);
          setInstalled(["installed", "pending-validation"].includes(result?.update?.outcome ?? ""));
        }
      }
    }
    catch (error) {
      if (mountedRef.current) {
        setInstallError(errorMessage(error));
        try {
          const result = await invokeCommand<MaintenanceStatus>("app_maintenance_status");
          if (mountedRef.current)
            setMaintenance(result ?? null);
        }
        catch (recoveryError) {
          if (mountedRef.current)
            setMaintenanceError(errorMessage(recoveryError));
        }
      }
    }
    finally {
      unlistenRef.current?.();
      unlistenRef.current = null;
      if (mountedRef.current)
        setDownloading(false);
    }
  }

  async function handleRestart() {
    setInstallError(null);
    try {
      await invokeCommand("restart_after_app_update");
    }
    catch (error) {
      setInstallError(errorMessage(error));
    }
  }

  async function handleRepairAcknowledgement() {
    if (!recovery)
      return;
    try {
      await invokeCommand("acknowledge_update_repair", { operationId: recovery.operationId });
      setMaintenance(await invokeCommand<MaintenanceStatus>("app_maintenance_status"));
      setInstallError(null);
    }
    catch (error) {
      setInstallError(errorMessage(error));
    }
  }

  async function handleManualDownload() {
    if (!status?.downloadUrl)
      return;
    setDownloadError(null);
    try {
      await openExternalUrl(status.downloadUrl);
    }
    catch (error) {
      setDownloadError(errorMessage(error));
    }
  }

  return (
    <div className="space-y-6">
      <SettingsSection description={t("update.description")}>
        <div className="space-y-3 rounded-lg border border-line-soft p-4">
          <div className="flex items-center gap-4">
            <div className="min-w-0 flex-1">
              <div className="text-sm font-medium text-ink">
                {build.data
                  ? t("update.currentVersion", { version: build.data.version })
                  : t("update.title")}
              </div>
            </div>
            <Button
              disabled={checking || downloading}
              leftIcon={<RefreshCw className={checking ? "size-3.5 animate-spin" : "size-3.5"} />}
              onClick={() => void handleCheck()}
              size="sm"
              variant="secondary"
            >
              {checking ? t("update.checking") : t("update.check")}
            </Button>
          </div>

          {maintenanceError ? <p role="alert" className="text-xs text-danger">{`${t("update.maintenanceUnavailable")}: ${maintenanceError}`}</p> : null}
          {maintenance?.cli.error ? <p role="alert" className="text-xs text-danger">{maintenance.cli.error}</p> : null}
          {maintenance?.cli.tool
            ? <p className="break-all text-xs text-ink-muted">{t("update.cliTool", maintenance.cli.tool)}</p>
            : null}
          {maintenance?.update
            ? (
                <div role="status" className="space-y-1 border-t border-line-soft pt-3 text-xs text-ink-muted">
                  <p>{t("update.operation", { id: maintenance.update.operationId, stage: t(`update.stages.${maintenance.update.stage}`, { defaultValue: maintenance.update.stage }), outcome: maintenance.update.outcome ? t(`update.outcomes.${maintenance.update.outcome}`, { defaultValue: maintenance.update.outcome }) : t("update.unconfirmed") })}</p>
                  {maintenance.update.diagnostic ? <p className="text-danger">{maintenance.update.diagnostic}</p> : null}
                  {!maintenance.update.outcome ? <p>{t("update.awaitingResult")}</p> : null}
                  {needsManualRepair && !downloading
                    ? <Button onClick={() => void handleRepairAcknowledgement()} size="sm" variant="secondary">{t("update.confirmRepair")}</Button>
                    : null}
                </div>
              )
            : null}
          {checkError ? <p className="text-xs text-danger">{`${t("update.checkFailed")}: ${checkError}`}</p> : null}

          {status && !checking
            ? (
                <div className="space-y-2 border-t border-line-soft pt-3">
                  {status.hasUpdate
                    ? (
                        <>
                          <p className="text-sm text-ink">
                            {t("update.available", { version: status.latestVersion })}
                          </p>

                          {status.canInstallInApp
                            ? (
                                <div className="space-y-2">
                                  {awaitingRestart
                                    ? (
                                        <div className="flex flex-wrap items-center gap-2">
                                          <span className="text-xs text-success">{t("update.installed")}</span>
                                          <Button
                                            leftIcon={<RotateCcw className="size-3.5" />}
                                            onClick={() => void handleRestart()}
                                            size="sm"
                                            variant="primary"
                                          >
                                            {t("update.restart")}
                                          </Button>
                                        </div>
                                      )
                                    : downloading
                                      ? (
                                          <div className="space-y-1.5">
                                            <div className="h-1.5 w-full overflow-hidden rounded-full bg-line-soft">
                                              <div
                                                className="h-full rounded-full bg-accent transition-[width]"
                                                style={{ width: `${progress}%` }}
                                              />
                                            </div>
                                            <div className="text-xs text-ink-muted">{preparing ? t("update.preparing") : installing ? t("update.installing") : `${t("update.downloading")} ${progress}%`}</div>
                                          </div>
                                        )
                                      : (
                                          <Button
                                            leftIcon={<Download className="size-3.5" />}
                                            disabled={needsRecovery || Boolean(maintenanceError) || needsManualRepair}
                                            onClick={() => void handleInstall()}
                                            size="sm"
                                            variant="primary"
                                          >
                                            {t("update.install")}
                                          </Button>
                                        )}
                                  {installError ? <p className="text-xs text-danger">{`${t("update.installFailed")}: ${installError}`}</p> : null}
                                  {installError && status.downloadUrl
                                    ? <Button onClick={() => void handleManualDownload()} size="sm" variant="secondary">{t("update.manualFallback")}</Button>
                                    : null}
                                  {downloadError ? <p role="alert" className="text-xs text-danger">{`${t("update.downloadFailed")}: ${downloadError}`}</p> : null}
                                  {needsRecovery && maintenance?.update?.restartSafe
                                    ? <Button onClick={() => void handleRestart()} size="sm" variant="primary">{t("update.restart")}</Button>
                                    : null}
                                </div>
                              )
                            : (
                                <div className="space-y-1">
                                  <div className="flex flex-wrap items-center gap-2">
                                    <span className="text-xs text-ink-muted">
                                      {t(status.platformSupported ? "update.manualOnly" : "update.noAsset")}
                                    </span>
                                    {status.downloadUrl
                                      ? (
                                          <a
                                            className="text-xs font-medium text-accent underline underline-offset-2"
                                            href={status.downloadUrl}
                                            onClick={(event) => {
                                              event.preventDefault();
                                              void handleManualDownload();
                                            }}
                                          >
                                            {t("update.download")}
                                          </a>
                                        )
                                      : null}
                                  </div>
                                  {downloadError ? <p className="text-xs text-danger">{`${t("update.downloadFailed")}: ${downloadError}`}</p> : null}
                                </div>
                              )}
                        </>
                      )
                    : <p className="text-sm text-ink-soft">{t("update.upToDate")}</p>}
                </div>
              )
            : null}
        </div>
      </SettingsSection>
    </div>
  );
}
