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

export function formatErrorType(errorType?: RunErrorType | null): { label: string; icon: string; color: string } | null {
  if (!errorType)
    return null;

  const errorTypes = {
    stream_disconnected: { label: i18n.t("runs:errorType.streamDisconnected"), icon: "🔌", color: "text-orange-600" },
    command_failed: { label: i18n.t("runs:errorType.commandFailed"), icon: "⚠️", color: "text-red-600" },
    model_failed: { label: i18n.t("runs:errorType.modelFailed"), icon: "🤖", color: "text-purple-600" },
    abort_requested: { label: i18n.t("runs:errorType.abortRequested"), icon: "⏹️", color: "text-gray-600" },
    timeout: { label: i18n.t("runs:errorType.timeout"), icon: "⏰", color: "text-yellow-600" },
    unknown: { label: i18n.t("runs:errorType.unknown"), icon: "❓", color: "text-gray-600" },
  };

  return errorTypes[errorType as keyof typeof errorTypes] || null;
}
