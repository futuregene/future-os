/**
 * Tasks as the phone sees them. The list row deliberately omits the prompt
 * body (the desktop's `list_tasks` never sends it); `getTask` fetches the full
 * record for the detail screen.
 */
export interface RemoteTaskRun {
  id: string;
  kind: string;
  origin: string;
  status: string;
  threadId?: string | null;
  /** The conversation this run ran in (absent on an older desktop, and
   * cleared when the task deletes its conversations). */
  sessionId?: string | null;
  /** The run's conversation was deleted after it settled (absent on an older
   * desktop). */
  sessionDeleted?: boolean;
  startedAt?: number | null;
  finishedAt?: number | null;
  promptVersion?: number | null;
  resultSummary?: string | null;
  errorMessage?: string | null;
}

export interface RemoteTaskRow {
  id: string;
  name: string;
  enabled: boolean;
  triggerKind: string;
  trigger: Record<string, unknown>;
  nextDueAt?: number | null;
  queued?: boolean;
  /**
   * Always 0 from a desktop that has been updated: task dependencies were
   * removed there. The field stays in the type because the desktop still sends
   * it (an older one sends a real count), and this app no longer acts on it.
   */
  depCount?: number;
  latestRun?: RemoteTaskRun | null;
}

export interface RemoteTaskDetail extends RemoteTaskRow {
  prompt: string;
  promptVersion: number;
  cwd: string;
  modelId?: string | null;
  thinkingLevel?: string | null;
  sessionPolicy: string;
  /** `keep` (default) or `delete`; only meaningful with `sessionPolicy: "new"`. */
  sessionRetention?: string;
  conversationMode?: string;
}
