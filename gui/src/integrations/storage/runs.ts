import type {
  StoredApprovalRequest,
  StoredRun,
  StoredRunEvent,
  StoredToolCall,
  StoredToolOutput,
} from "./types";
import { invokeCommand } from "../tauri/invoke";

// ─── Runs ────────────────────────────────────────────────────────────────

export async function createRun(input: {
  threadId: string;
  triggerMessageId?: string | null;
  modelProvider?: string | null;
  modelId?: string | null;
}) {
  return invokeCommand<StoredRun>("create_run", { input });
}

export async function listRuns(threadId: string) {
  return invokeCommand<StoredRun[]>("list_runs", { threadId });
}

/** The thread's most recent run, or null (recent-run poll — one row). */
export async function getLatestRun(threadId: string) {
  return invokeCommand<StoredRun | null>("get_latest_run", { threadId });
}

/** A single run by id (settle checks — direct PK lookup). */
export async function getRun(runId: string) {
  return invokeCommand<StoredRun | null>("get_run", { runId });
}

export async function listLatestRunInfos(threadIds: string[]) {
  return invokeCommand<Array<{ threadId: string; status: string; endedAt: number | null }>>(
    "list_latest_run_infos",
    { threadIds },
  );
}

export async function updateRunStatus(input: {
  runId: string;
  status: StoredRun["status"];
  errorMessage?: string | null;
}) {
  return invokeCommand<StoredRun>("update_run_status", { input });
}

export async function abortRun(input: { threadId: string; runId: string }) {
  return invokeCommand<StoredRun>("abort_run", { threadId: input.threadId, runId: input.runId });
}

export async function clearFinishedRuns(threadId: string) {
  return invokeCommand<number>("clear_finished_runs", { threadId });
}

export async function listRunEvents(runId: string) {
  return invokeCommand<StoredRunEvent[]>("list_run_events", { runId });
}

/**
 * Incremental variant for the live-preview poll: only events with
 * `sequence > sinceSequence` cross IPC. Pass -1 for the full log.
 */
export async function listRunEventsSince(runId: string, sinceSequence: number) {
  return invokeCommand<StoredRunEvent[]>("list_run_events_since", { runId, sinceSequence });
}

/** Fetch run events for multiple runs in a single IPC call. */
export async function listRunEventsBulk(runIds: string[]) {
  return invokeCommand<[string, StoredRunEvent[]][]>("list_run_events_bulk", { runIds });
}

export async function listToolCalls(runId: string) {
  return invokeCommand<StoredToolCall[]>("list_tool_calls", { runId });
}

/** Tool calls for many runs in a single IPC call (context panel poll). */
export async function listToolCallsBulk(runIds: string[]) {
  return invokeCommand<[string, StoredToolCall[]][]>("list_tool_calls_bulk", { runIds });
}

export async function listToolOutputs(runId: string, toolCallId: string) {
  return invokeCommand<StoredToolOutput[]>("list_tool_outputs", { runId, toolCallId });
}

// ─── Approvals ───────────────────────────────────────────────────────────

export async function listApprovalRequests(threadId: string) {
  return invokeCommand<StoredApprovalRequest[]>("list_approval_requests", { threadId });
}

export async function decideApprovalRequest(input: {
  approvalRequestId: string;
  status: "approved" | "rejected" | "cancelled";
  decisionNote?: string | null;
}) {
  return invokeCommand<StoredApprovalRequest>("decide_approval_request", { input });
}

export async function saveApprovalRule(input: {
  threadId: string;
  path: string;
  access: string; // "read" | "write"
}) {
  return invokeCommand<void>("save_approval_rule", { input });
}
