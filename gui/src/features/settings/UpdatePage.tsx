import { listen } from "@tauri-apps/api/event";
import { Download, FolderOpen, RefreshCw } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "../../components/ui/Button";
import { invokeCommand } from "../../integrations/tauri/invoke";
import { useBuildInfo } from "../../integrations/tauri/useBuildInfo";
import { SettingsSection } from "./SettingsPrimitives";

/** Mirrors the backend `UpdateStatus` (serde camelCase). */
interface UpdateStatus {
  currentVersion: string;
  latestVersion: string;
  hasUpdate: boolean;
  platformSupported: boolean;
  downloadUrl: string | null;
  fileName: string | null;
}

interface DownloadProgress {
  downloaded: number;
  total: number;
}

/**
 * Software update page: check the OSS release manifest and download the
 * installer for this platform. Dev builds always report an update available.
 */
export function UpdatePage() {
  const { t } = useTranslation("settings");
  const build = useBuildInfo();
  const [status, setStatus] = useState<UpdateStatus | null>(null);
  const [checking, setChecking] = useState(false);
  const [checkError, setCheckError] = useState<string | null>(null);
  const [downloading, setDownloading] = useState(false);
  const [progress, setProgress] = useState(0);
  const [savedPath, setSavedPath] = useState<string | null>(null);
  const [downloadError, setDownloadError] = useState<string | null>(null);

  async function handleCheck() {
    setChecking(true);
    setCheckError(null);
    setSavedPath(null);
    setDownloadError(null);
    try {
      setStatus(await invokeCommand<UpdateStatus>("check_app_update"));
    }
    catch (error) {
      setStatus(null);
      setCheckError(error instanceof Error ? error.message : String(error));
    }
    finally {
      setChecking(false);
    }
  }

  async function handleDownload() {
    if (!status?.downloadUrl || !status.fileName)
      return;
    setDownloading(true);
    setProgress(0);
    setDownloadError(null);
    setSavedPath(null);
    // Listen for streamed progress before kicking off the download.
    const unlisten = await listen<DownloadProgress>("app-update-progress", (event) => {
      const { downloaded, total } = event.payload;
      if (total > 0)
        setProgress(Math.min(100, Math.round((downloaded / total) * 100)));
    });
    try {
      const path = await invokeCommand<string>("download_app_update", {
        url: status.downloadUrl,
        fileName: status.fileName,
      });
      setSavedPath(path);
    }
    catch (error) {
      setDownloadError(error instanceof Error ? error.message : String(error));
    }
    finally {
      unlisten();
      setDownloading(false);
    }
  }

  async function handleReveal() {
    if (!savedPath)
      return;
    // Open the containing folder (strip the trailing path segment, either sep).
    const dir = savedPath.replace(/[/\\][^/\\]*$/, "");
    await invokeCommand("open_path", { path: dir });
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

                          {status.platformSupported
                            ? (
                                <div className="space-y-2">
                                  {savedPath
                                    ? (
                                        <div className="flex flex-wrap items-center gap-2">
                                          <span className="text-xs text-success">{t("update.downloaded")}</span>
                                          <Button
                                            leftIcon={<FolderOpen className="size-3.5" />}
                                            onClick={() => void handleReveal()}
                                            size="sm"
                                            variant="secondary"
                                          >
                                            {t("update.reveal")}
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
                                            <div className="text-xs text-ink-muted">{`${t("update.downloading")} ${progress}%`}</div>
                                          </div>
                                        )
                                      : (
                                          <Button
                                            leftIcon={<Download className="size-3.5" />}
                                            onClick={() => void handleDownload()}
                                            size="sm"
                                            variant="primary"
                                          >
                                            {t("update.download")}
                                          </Button>
                                        )}
                                  {downloadError ? <p className="text-xs text-danger">{`${t("update.downloadFailed")}: ${downloadError}`}</p> : null}
                                </div>
                              )
                            : <p className="text-xs text-ink-muted">{t("update.noAsset")}</p>}
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
