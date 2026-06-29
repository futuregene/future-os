export type AgentMode = "plan" | "research" | "build" | "review";

export type MessageRole = "user" | "assistant" | "system";

export interface ToolCall {
  id: string;
  name: string;
  status: "queued" | "running" | "completed" | "failed";
  summary: string;
  input: string;
  output?: string;
}

export interface AgentPlanStep {
  id: string;
  title: string;
  detail: string;
  status: "completed" | "active" | "pending";
}

export type AgentActivityKind = "thinking" | "read" | "bash" | "edit" | "write";

export interface AgentActivityItem {
  id: string;
  kind: AgentActivityKind;
  status: "running" | "completed" | "failed";
  target?: string;
  detail?: string;
  count?: number;
  additions?: number;
  deletions?: number;
}

/**
 * One ordered slice of an assistant turn. Text and tool activity are kept in
 * the chronological order the agent produced them (Claude-style inline tool
 * calls), instead of being flattened into "all text, then all tools".
 */
export type MessageSegment
  = | { kind: "text"; id: string; text: string }
    | { kind: "activity"; id: string; item: AgentActivityItem };

export interface MessageAttachment {
  artifactId?: string | null;
  name: string;
  path: string;
  /** image | pdf | text — drives inlining and thread rendering. */
  kind?: "image" | "pdf" | "text" | null;
  /** Absolute path to a cached thumbnail (images only), rendered via convertFileSrc. */
  thumbnail?: string | null;
}

export interface AgentMessage {
  id: string;
  runId?: string | null;
  role: MessageRole;
  author: string;
  content: string;
  status?: "complete" | "streaming" | "failed";
  createdAt: string;
  activityItems?: AgentActivityItem[];
  /**
   * Ordered text/activity slices for inline rendering. Falls back to
   * content + activityItems when absent (optimistic, error, legacy data).
   */
  segments?: MessageSegment[];
  attachments?: MessageAttachment[];
  plan?: AgentPlanStep[];
  toolCalls?: ToolCall[];
  /**
   * Model id of the run that produced this assistant turn (resolved to a
   * display label at render time).
   */
  modelId?: string | null;
  /** Epoch ms anchor for the live elapsed timer while streaming. */
  runStartedAt?: number | null;
  /** Final model run duration (ms), set once the run settles. */
  durationMs?: number | null;
  /** Tokens this reply generated (summed completion tokens across the run). */
  outputTokens?: number | null;
}
