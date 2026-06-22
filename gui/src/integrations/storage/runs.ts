import type {
  StoredApprovalRequest,
  StoredRun,
  StoredRunEvent,
  StoredToolCall,
  StoredToolOutput,
} from "./types";
import { invoke } from "@tauri-apps/api/core";

// ─── Runs ────────────────────────────────────────────────────────────────

export async function createRun(input: {
  threadId: string;
  triggerMessageId?: string | null;
  modelProvider?: string | null;
  modelId?: string | null;
}) {
  return invoke<StoredRun>("create_run", { input });
}

export async function listRuns(threadId: string) {
  return invoke<StoredRun[]>("list_runs", { threadId });
}

export async function updateRunStatus(input: {
  runId: string;
  status: StoredRun["status"];
  errorMessage?: string | null;
}) {
  return invoke<StoredRun>("update_run_status", { input });
}

export async function abortRun(input: { threadId: string; runId: string }) {
  return invoke<StoredRun>("abort_run", input);
}

export async function clearFinishedRuns(threadId: string) {
  return invoke<number>("clear_finished_runs", { threadId });
}

export async function listRunEvents(runId: string) {
  return invoke<StoredRunEvent[]>("list_run_events", { runId });
}

export async function listToolCalls(runId: string) {
  return invoke<StoredToolCall[]>("list_tool_calls", { runId });
}

export async function listToolOutputs(toolCallId: string) {
  return invoke<StoredToolOutput[]>("list_tool_outputs", { toolCallId });
}

// ─── Approvals ───────────────────────────────────────────────────────────

export async function cancelStaleApprovalRequests() {
  return invoke<number>("cancel_stale_approval_requests");
}

export async function listApprovalRequests(threadId: string) {
  return invoke<StoredApprovalRequest[]>("list_approval_requests", { threadId });
}

export async function decideApprovalRequest(input: {
  approvalRequestId: string;
  status: "approved" | "rejected" | "cancelled";
  decisionNote?: string | null;
}) {
  return invoke<StoredApprovalRequest>("decide_approval_request", { input });
}
