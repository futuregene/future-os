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
