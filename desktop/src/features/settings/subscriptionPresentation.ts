import type { FutureTokenPlan } from "../../integrations/agent/providers";

/** i18n key for a subscription lifecycle status. */
export const PLAN_STATUS_KEYS: Record<NonNullable<FutureTokenPlan["status"]>, string> = {
  active: "account.planStatusActive",
  scheduled: "account.planStatusScheduled",
  expired: "account.planStatusExpired",
  canceled: "account.planStatusCanceled",
};

/**
 * A credit amount without trailing zeros: `5`, `1.25`, `0.5`. The platform
 * sends quotas as floating-point credits, and `5.0000000001`-style artifacts
 * would otherwise read as a precision the number does not have.
 */
export function formatCredits(value: number): string {
  if (!Number.isFinite(value))
    return "0";
  return String(Math.round(value * 100) / 100);
}

/**
 * How full the current window's quota bar is, as a 0–100 percentage. A window
 * that grants nothing (no quota yet) shows an empty bar rather than dividing by
 * zero; an overrun clamps to full, with the caller styling it separately.
 */
export function windowUsagePercent(granted: number, used: number): number {
  if (!Number.isFinite(granted) || !Number.isFinite(used) || granted <= 0)
    return 0;
  return Math.min(100, Math.max(0, (used / granted) * 100));
}
