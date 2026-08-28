export interface StoredThread {
  id: string;
  workspaceId: string;
  mode: "chat" | "workspace";
  title: string;
  status: "active" | "archived" | "deleted";
  pinned: boolean;
  readonly: boolean;
  // modelProvider, modelId, thinkingLevel removed — now from agent state cache
  agentSessionId?: string | null;
  lastMessageAt?: number | null;
  lastOpenedAt?: number | null;
  createdAt: number;
  updatedAt: number;
}

export interface StoredWorkspace {
  id: string;
  name: string;
  kind: "user" | "temporary";
  path: string;
  description?: string | null;
  cleanupStatus: "active" | "pending_cleanup" | "cleaned";
  cleanupRequestedAt?: number | null;
  cleanedAt?: number | null;
  lastOpenedAt?: number | null;
  createdAt: number;
  updatedAt: number;
  deletedAt?: number | null;
}

export interface StoredMessage {
  id: string;
  threadId: string;
  runId?: string | null;
  role: "user" | "assistant" | "system" | "tool";
  contentType: "text" | "markdown" | "mixed";
  content: string;
  status: "complete" | "streaming" | "failed";
  createdAt: number;
  updatedAt: number;
}

export interface StoredRun {
  id: string;
  threadId: string;
  triggerMessageId?: string | null;
  status: "queued" | "running" | "waiting_approval" | "completed" | "failed" | "cancelled";
  modelProvider?: string | null;
  modelId?: string | null;
  startedAt?: number | null;
  endedAt?: number | null;
  errorMessage?: string | null;
  errorType?: "stream_disconnected" | "command_failed" | "model_failed" | "abort_requested" | "timeout" | "unknown" | null;
  /** Set when hidden from the Runs panel; the record and Agent events remain available. */
  archivedAt?: number | null;
  createdAt: number;
  updatedAt: number;
}

export interface StoredRunEvent {
  id: string;
  runId: string;
  eventType: string;
  payload?: string | null;
  sequence: number;
  createdAt: number;
}

export interface StoredToolCall {
  id: string;
  runId: string;
  name: string;
  kind: string;
  input?: string | null;
  status: string;
  startedAt?: number | null;
  endedAt?: number | null;
  createdAt: number;
}

export interface StoredToolOutput {
  id: string;
  toolCallId: string;
  kind: string;
  content?: string | null;
  createdAt: number;
}

export interface StoredApprovalRequest {
  id: string;
  threadId: string;
  runId?: string | null;
  toolCallId?: string | null;
  kind: string;
  status: "pending" | "approved" | "rejected" | "cancelled" | string;
  title: string;
  summary?: string | null;
  riskLevel?: string | null;
  requestedAction?: string | null;
  decisionNote?: string | null;
  decidedAt?: number | null;
  createdAt: number;
  updatedAt: number;
  // P2: structured action and sandbox boundary
  actionCategory?: string | null;
  actionPayload?: string | null;
  sandboxBoundary?: string | null;
  // Phase 2: suggested rule (JSON) for session/always-allow persistence.
  saveSuggestion?: string | null;
  reviewer: string;
  decisionScope: string;
  decisionSource: string;
}

// v2: parsed save_suggestion / P2 structured action payload — moved to the
// shared `@future-os/thread-projection` package (`src/approval.ts`) so the
// approval semantic model is single-sourced across desktop and mobile.

export interface StoredReviewChangeset {
  id: string;
  threadId: string;
  runId?: string | null;
  toolCallId?: string | null;
  title: string;
  summary?: string | null;
  status: "applied" | "discarded" | "pending" | string;
  filesChanged: number;
  additions: number;
  deletions: number;
  // Shadow review (source_kind = 'run_snapshot') fields — see desktop/ER.md §4.10.
  sourceKind: string;
  workspaceId?: string | null;
  beforeSnapshotId?: string | null;
  afterSnapshotId?: string | null;
  binaryFiles: number;
  omittedFiles: number;
  completeness: "complete" | "partial" | string;
  confidence: "normal" | "recovered" | string;
  overlapped: boolean;
  errorMessage?: string | null;
  createdAt: number;
  updatedAt: number;
}

export interface StoredReviewFileChange {
  id: string;
  changesetId: string;
  targetType: string;
  targetId?: string | null;
  path?: string | null;
  changeType: string;
  beforeRef?: string | null;
  afterRef?: string | null;
  diff?: string | null;
  summary?: string | null;
  additions: number;
  deletions: number;
  // Shadow review fields — see desktop/ER.md §4.10.
  previousPath?: string | null;
  binary: boolean;
  beforeSize?: number | null;
  afterSize?: number | null;
  mime?: string | null;
  diffTruncated: boolean;
  omissionReason?: string | null;
  createdAt: number;
  updatedAt: number;
}

/// Workspace review capabilities (§10.1).
export interface WorkspaceReviewCapabilities {
  isGitWorkspace: boolean;
  views: Array<"git_changes" | "last_run">;
  defaultView: "git_changes" | "last_run";
  changePreview: "ready" | "unsupported_too_large";
}

/// The "last-run changes" payload for a Thread (§10.3).
export interface LastRunReviewData {
  changeset: StoredReviewChangeset;
  files: StoredReviewFileChange[];
  run?: StoredRun | null;
  snapshotStatus: "complete" | "partial" | "incomplete" | "unavailable";
  confidence: "normal" | "recovered";
  overlapped: boolean;
}

export interface GitReview {
  isGitWorkspace: boolean;
  workspacePath: string;
  branch?: string | null;
  upstream?: string | null;
  diffBase?: string | null;
  diffBaseLabel?: string | null;
  additions: number;
  deletions: number;
  files: GitReviewFile[];
}

export interface GitReviewFile {
  path: string;
  status: string;
  additions: number;
  deletions: number;
  diff: string;
  binary: boolean;
  diffTruncated: boolean;
  omissionReason?: string | null;
}

export interface ThreadCleanupSummary {
  threadId: string;
  workspaceId: string;
  workspaceKind: "temporary" | "user";
  workspacePath: string;
  cleanupStatus: "active" | "pending_cleanup" | "cleaned";
  artifactCount: number;
  workspaceFileCount: number;
}

export interface StoredArtifact {
  id: string;
  workspaceId: string;
  threadId?: string | null;
  runId?: string | null;
  title: string;
  artifactType: string;
  path?: string | null;
  content?: string | null;
  contentStorage?: "file" | "inline" | string | null;
  summary?: string | null;
  createdAt: number;
  updatedAt: number;
  deletedAt?: number | null;
}

/// A local-file link (a plain markdown path link, e.g. `[name](/abs/path)`),
/// resolved to a display model by pure path arithmetic (no filesystem access).
/// See `resolve.rs::ResolvedFile`.
export interface StoredFile {
  /** Absolute path, used for open / copy-path actions. */
  path: string;
  /** File name (last path component). */
  name: string;
  /** Path relative to the workspace root; present only when inside it. */
  relativePath?: string | null;
  insideWorkspace: boolean;
}

export interface WorkspaceFileResult {
  /** Path relative to the workspace root (POSIX separators). */
  path: string;
  /** Last path component, for display emphasis. */
  name: string;
}
