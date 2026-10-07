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
  startedAt?: number | null;
  finishedAt?: number | null;
  promptVersion?: number | null;
  resultSummary?: string | null;
  errorMessage?: string | null;
  /** Whether this run can be forked into the session list. */
  forkable?: boolean;
}

export interface RemoteTaskRow {
  id: string;
  name: string;
  enabled: boolean;
  triggerKind: string;
  trigger: Record<string, unknown>;
  nextDueAt?: number | null;
  lastRunAt?: number | null;
  reflection: string;
  latestRun?: RemoteTaskRun | null;
}

export interface RemoteTaskDetail extends RemoteTaskRow {
  prompt: string;
  promptVersion: number;
  cwd: string;
  modelId?: string | null;
  thinkingLevel?: string | null;
  sessionPolicy: string;
  conversationMode?: string;
  depJoin: string;
}

export interface RemoteTaskDep {
  upstreamTaskId: string;
  upstreamName: string;
  on: string;
  satisfied: boolean;
}

export interface RemoteTaskRevision {
  id: string;
  version: number;
  source: string;
  status: string;
  reason?: string | null;
  confidence?: number | null;
  createdAt: number;
  promptPreview: string;
}
