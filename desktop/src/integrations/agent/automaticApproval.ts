import type { ApprovalTier } from "../storage/appSettings";

/** A temporary account verification failure does not mean the user signed out. */
export function automaticApprovalAvailable(sessionStatus: string): boolean {
  return sessionStatus === "authenticated" || sessionStatus === "unavailable";
}

export function effectiveApprovalTier(tier: ApprovalTier, sessionStatus: string): ApprovalTier {
  return shouldPersistAutomaticApprovalFallback(tier, sessionStatus) ? "sandbox" : tier;
}

export function shouldPersistAutomaticApprovalFallback(tier: ApprovalTier, sessionStatus: string): boolean {
  return tier === "auto" && (sessionStatus === "signed_out" || sessionStatus === "invalid");
}
