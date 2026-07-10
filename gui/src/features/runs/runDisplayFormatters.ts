import type { StoredRun } from "../../integrations/storage/threadStore";
import i18n from "../../i18n";

export function formatRunStatus(status: StoredRun["status"]) {
  switch (status) {
    case "completed":
      return i18n.t("runs:status.completed");
    case "failed":
      return i18n.t("runs:status.failed");
    case "running":
      return i18n.t("runs:status.running");
    case "waiting_approval":
      return i18n.t("runs:status.approval");
    case "cancelled":
      return i18n.t("runs:status.cancelled");
    default:
      return i18n.t("runs:status.queued");
  }
}

/** Title-case status label for run rows (badges use the lowercase `formatRunStatus`). */
export function runStatusLabel(status: StoredRun["status"]) {
  switch (status) {
    case "completed":
      return i18n.t("runs:statusLabel.success");
    case "failed":
      return i18n.t("runs:statusLabel.failed");
    case "cancelled":
      return i18n.t("runs:statusLabel.cancelled");
    case "waiting_approval":
      return i18n.t("runs:statusLabel.waiting");
    case "queued":
      return i18n.t("runs:statusLabel.queued");
    default:
      return i18n.t("runs:statusLabel.running");
  }
}

/** Localized label for a tool call's own status (not the enclosing run's). */
export function toolStatusLabel(status: string) {
  switch (status) {
    case "completed":
      return i18n.t("runs:toolStatus.completed");
    case "failed":
      return i18n.t("runs:toolStatus.failed");
    case "cancelled":
      return i18n.t("runs:toolStatus.cancelled");
    case "running":
      return i18n.t("runs:toolStatus.running");
    default:
      return status || i18n.t("runs:toolStatus.unknown");
  }
}

export function runTone(status: StoredRun["status"]) {
  if (status === "completed")
    return "success";
  if (status === "failed" || status === "cancelled")
    return "danger";
  if (status === "waiting_approval")
    return "warning";
  if (status === "running")
    return "accent";
  return "neutral";
}

export function shortId(id: string) {
  return id.split("_").slice(0, 2).join("_");
}

export type RunErrorType = NonNullable<StoredRun["errorType"]>;
