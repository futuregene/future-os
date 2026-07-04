export interface StoredThread {
  id: string;
  workspaceId: string;
  mode: "chat" | "workspace";
  title: string;
  status: "active" | "archived" | "deleted";
  pinned: boolean;
  readonly: boolean;
  modelProvider?: string | null;
  modelId?: string | null;
  thinkingLevel?: string | null;
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

// v2: parsed save_suggestion — the file rule to persist on "allow in this
// workspace". `path` is a glob (workspace-relative, or ~/absolute).
export interface ApprovalSaveSuggestion {
  path: string;
  access: string; // "read" | "write"
}

// P2: structured action payload (parsed from actionPayload JSON)
export interface ApprovalAction {
  tool: string;
  category: string;
  summary?: string;
  command?: string;
  paths?: string[];
  writes?: Array<{ path: string; preview?: string }>;
  deletes?: Array<{ path: string }>;
  // sandbox_escalation: model-provided reason and the file paths the sandbox
  // blocked (extracted from the failed run — no raw stderr dump).
  justification?: string;
  blockedPaths?: string[];
  scope?: {
    cwd: string;
    insideWorkspace: boolean;
    estimatedBlastRadius: "low" | "medium" | "high";
  };
}

// P2: sandbox boundary info (parsed from sandboxBoundary JSON)
export interface SandboxBoundary {
  mode: string;
  insideSandbox: boolean;
  violation?: string | null;
  cwd: string;
  writableRoots?: string[];
}

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
  // Shadow review (source_kind = 'run_snapshot') fields — see gui/ER.md §4.10.
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
  // Shadow review fields — see gui/ER.md §4.10.
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

/// The "上一轮变更" payload for a Thread (§10.3).
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

/// A `futureos://file/<path>` reference resolved against a workspace file on
/// disk (not the artifacts table). See `resolve.rs::ResolvedFile`.
export interface StoredFile {
  path: string;
  name: string;
  artifactType: string;
}

export interface StoredResearchResource {
  id: string;
  collectionId: string;
  workspaceId: string;
  sourceArtifactId?: string | null;
  title: string;
  resourceType: string;
  sourceUri?: string | null;
  content?: string | null;
  contentStorage?: "file" | "inline" | string | null;
  summary?: string | null;
  metadata?: string | null;
  createdAt: number;
  updatedAt: number;
}

export interface ReferenceTargetSearchResult {
  targetType: "approval" | "artifact" | "research" | "review" | "run" | "tool" | string;
  targetId: string;
  title: string;
  subtitle?: string | null;
  searchText?: string | null;
  updatedAt: number;
}
