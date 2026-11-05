import type { MaintenanceStatus } from "../../integrations/tauri/useMaintenanceStatus";

// The official site remains reachable even when update discovery is unavailable.
export const UPDATE_DOWNLOAD_PAGE = "https://future-os.cn/";

/**
 * When the result on screen was produced, in the reader's own clock.
 *
 * A cached result is still a real result — the desktop checks once a day in the
 * background — but the page must not present it as "now": "you're on the latest
 * version" with no time reads as if a check had just run on this visit. Same-day
 * results read as `HH:mm`; an older one carries its date.
 */
export function formatCheckedAt(checkedAt: number, now = Date.now()) {
  const then = new Date(checkedAt);
  const today = new Date(now);
  const pad = (value: number) => String(value).padStart(2, "0");
  const time = `${pad(then.getHours())}:${pad(then.getMinutes())}`;
  const sameDay = then.getFullYear() === today.getFullYear()
    && then.getMonth() === today.getMonth()
    && then.getDate() === today.getDate();
  return sameDay ? time : `${pad(then.getMonth() + 1)}-${pad(then.getDate())} ${time}`;
}

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
  // A failed pre-install cleanup records no installer handoff, so the journal
  // accepts a new attempt (`Operation::begin` only refuses unconfirmed,
  // repair-required and interrupted-mid-install records). The retryable flag
  // separates "this is a problem" from "this problem blocks installing":
  // cleanup-failed and failed-restored may retry, the rest may not.
  const retryable = Boolean(outcome && ["cleanup-failed", "failed-restored"].includes(outcome));
  return { repair, ready, working, canRestart, problem, retryable };
}
