import type { StoredRun } from "../../../integrations/storage/threadStore";

export function formatRunStatus(status: StoredRun["status"]) {
  switch (status) {
    case "completed":
      return "completed";
    case "failed":
      return "failed";
    case "running":
      return "running";
    case "waiting_approval":
      return "approval";
    case "cancelled":
      return "cancelled";
    default:
      return "queued";
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

export function summarizePayload(payload: string) {
  try {
    const value = JSON.parse(payload) as unknown;
    return JSON.stringify(value, null, 2).slice(0, 1200);
  }
  catch {
    return payload.slice(0, 1200);
  }
}

export type RunErrorType = NonNullable<StoredRun["errorType"]>;

export function formatErrorType(errorType?: RunErrorType | null): { label: string; icon: string; color: string } | null {
  if (!errorType)
    return null;

  const errorTypes = {
    stream_disconnected: { label: "Stream disconnected", icon: "🔌", color: "text-orange-600" },
    command_failed: { label: "Command failed", icon: "⚠️", color: "text-red-600" },
    model_failed: { label: "Model failed", icon: "🤖", color: "text-purple-600" },
    abort_requested: { label: "Aborted by user", icon: "⏹️", color: "text-gray-600" },
    timeout: { label: "Timeout", icon: "⏰", color: "text-yellow-600" },
    unknown: { label: "Unknown error", icon: "❓", color: "text-gray-600" },
  };

  return errorTypes[errorType as keyof typeof errorTypes] || null;
}
