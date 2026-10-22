import type { UpdateStatus } from "../../components/layout/hooks/useUpdateChecker";
import { Loader2 } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "../../components/ui/Button";
import { openExternalUrl } from "../../integrations/storage/files";
import { invokeCommand } from "../../integrations/tauri/invoke";
import { useMaintenanceStatus } from "../../integrations/tauri/useMaintenanceStatus";
import { UPDATE_DOWNLOAD_PAGE, updatePresentation } from "./updatePresentation";

/** Remains available when shutdown has unmounted all Agent-dependent UI. */
export function UpdateRecoveryPanel({ shutdownFailed = false }: { shutdownFailed?: boolean }) {
  const { t } = useTranslation("settings");
  const { maintenance, maintenanceError } = useMaintenanceStatus();
  const [error, setError] = useState<"update.restartFailed" | "update.downloadFailed" | null>(null);
  const [busy, setBusy] = useState(false);
  const result = maintenance?.update;
  const { repair, ready, canRestart, problem } = updatePresentation(result ?? null);
  const message = maintenanceError
    ? "update.maintenanceUnavailable"
    : shutdownFailed
      ? "update.cleanupFailed"
      : problem ?? (ready || result?.outcome === "installed" ? "update.installed" : "update.awaitingResult");
  const working = !maintenanceError && !shutdownFailed && !problem && !canRestart;

  async function restart() {
    setBusy(true);
    setError(null);
    try {
      await invokeCommand("restart_after_app_update");
    }
    catch {
      setError("update.restartFailed");
    }
    finally {
      setBusy(false);
    }
  }

  async function download() {
    setBusy(true);
    setError(null);
    try {
      // A failed check must not leave a stopped app without a recovery link.
      const update = await invokeCommand<UpdateStatus>("check_app_update").catch(() => null);
      await openExternalUrl(update?.downloadUrl ?? UPDATE_DOWNLOAD_PAGE);
    }
    catch {
      setError("update.downloadFailed");
    }
    finally {
      setBusy(false);
    }
  }

  return (
    <div className="flex flex-col items-center gap-3 text-sm" aria-live="polite">
      {working ? <Loader2 aria-hidden="true" className="size-6 animate-spin text-accent" /> : null}
      <p className="text-lg font-medium">{t("update.title")}</p>
      <p role={problem || maintenanceError || shutdownFailed ? "alert" : "status"} className="text-ink-muted">{t(message)}</p>
      {error ? <p role="alert" className="text-ink-muted">{t(error)}</p> : null}
      {canRestart && !shutdownFailed && !maintenanceError ? <Button disabled={busy} onClick={() => void restart()}>{t("update.restart")}</Button> : null}
      {repair || shutdownFailed || maintenanceError || error === "update.restartFailed"
        ? <Button disabled={busy} onClick={() => void download()} variant="secondary">{t("update.manualFallback")}</Button>
        : null}
    </div>
  );
}
