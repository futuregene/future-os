export type MessageRole = "user" | "assistant" | "system";

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
  /**
   * The individual tool calls a collapsed summary row stands for (e.g. the 4
   * commands behind "Ran 4 commands"). Present only on grouped items; each child
   * is a leaf item carrying its own target/detail. Drives the row's inline
   * preview and its expandable sub-list.
   */
  children?: AgentActivityItem[];
}

/**
 * One ordered slice of an assistant turn. Text and tool activity are kept in
 * the chronological order the agent produced them (Claude-style inline tool
 * calls), instead of being flattened into "all text, then all tools".
 */
export type MessageSegment
  = | { kind: "text"; id: string; text: string }
    | { kind: "thinking"; id: string; text: string }
    | { kind: "activity"; id: string; item: AgentActivityItem }
    // A context-compaction marker (history summarized mid-run). `tokensBefore` is
    // the pre-compaction token count when the agent reported one (0/omitted for
    // the retry-path compaction, which carries no count).
    | { kind: "compaction"; id: string; tokensBefore?: number };

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
  /**
   * i18n key (in the `agent` namespace) for the author, e.g. `author.you`. It is
   * resolved at render time so the author label follows the active language even
   * for messages already in state — never pre-resolve it in the logic layer.
   */
  authorKey: string;
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
  /** The reply was interrupted by the user (its run was cancelled mid-stream). */
  stopped?: boolean;
  /**
   * The model is mid-reasoning with nothing visible yet. Drives the footer
   * "thinking…" hint (only while streaming and the show-thinking setting is off).
   */
  thinkingActive?: boolean;
}
