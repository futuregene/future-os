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
}

export interface StoredReviewChangeset {
  id: string;
  threadId: string;
  runId?: string | null;
  toolCallId?: string | null;
  title: string;
  summary?: string | null;
  status: string;
  filesChanged: number;
  additions: number;
  deletions: number;
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
  createdAt: number;
  updatedAt: number;
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
