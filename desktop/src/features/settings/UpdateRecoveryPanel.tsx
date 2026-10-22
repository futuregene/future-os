import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "../../components/ui/Button";
import { invokeCommand } from "../../integrations/tauri/invoke";
import { useMaintenanceStatus } from "../../integrations/tauri/useMaintenanceStatus";
import { errorMessage } from "../../lib/errors";

/** Remains available when shutdown has unmounted all Agent-dependent UI. */
export function UpdateRecoveryPanel() {
  const { t } = useTranslation("settings");
  const { maintenance, maintenanceError } = useMaintenanceStatus();
  const [error, setError] = useState<string | null>(null);
  const [restarting, setRestarting] = useState(false);
  const result = maintenance?.update;
  const canRestart = result?.restartSafe && ["installed", "pending-validation", "failed-restored", "not-installed"].includes(result.outcome ?? "");

  async function restart() {
    setRestarting(true);
    setError(null);
    try {
      await invokeCommand("restart_after_app_update");
    }
    catch (failure) {
      setError(errorMessage(failure));
    }
    finally {
      setRestarting(false);
    }
  }

  return (
    <div className="space-y-3 text-sm" aria-live="polite">
      {result ? <p>{t("update.operation", { id: result.operationId, stage: t(`update.stages.${result.stage}`, { defaultValue: result.stage }), outcome: result.outcome ? t(`update.outcomes.${result.outcome}`, { defaultValue: result.outcome }) : t("update.unconfirmed") })}</p> : null}
      {result?.diagnostic ? <p role="alert">{result.diagnostic}</p> : null}
      {result && !result.outcome ? <p>{t("update.awaitingResult")}</p> : null}
      {maintenanceError || error ? <p role="alert">{maintenanceError ?? error}</p> : null}
      {canRestart ? <Button disabled={restarting} onClick={() => void restart()}>{t("update.restart")}</Button> : null}
    </div>
  );
}
