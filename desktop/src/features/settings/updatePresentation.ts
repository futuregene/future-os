import type { MaintenanceStatus } from "../../integrations/tauri/useMaintenanceStatus";

// The official site remains reachable even when update discovery is unavailable.
export const UPDATE_DOWNLOAD_PAGE = "https://future-os.cn/";

/** User-facing states deliberately exclude journal identifiers and diagnostics. */
export function updatePresentation(result: MaintenanceStatus["update"]) {
  const outcome = result?.outcome;
  const repair = Boolean(result && !result.repairAcknowledged && (outcome === "repair-required" || (outcome === "interrupted" && ["installing", "awaiting-exit"].includes(result.stage))));
  const ready = outcome === "pending-validation";
  const working = Boolean(result && !outcome);
  const canRestart = Boolean(result?.restartSafe && ["installed", "pending-validation", "failed-restored", "not-installed"].includes(outcome ?? ""));
  const problem = repair
    ? "update.repairRequired"
    : outcome === "cleanup-failed"
      ? "update.cleanupFailed"
      : outcome === "failed-restored"
        ? "update.restored"
        : ["failed", "interrupted", "not-installed"].includes(outcome ?? "")
            ? "update.installFailed"
            : null;
  return { repair, ready, working, canRestart, problem };
}
