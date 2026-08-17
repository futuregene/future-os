export type ConnectionPhase =
  | "booting"
  | "unpaired"
  | "claiming"
  | "connecting"
  | "connected"
  | "reconnecting"
  | "refreshing"
  | "failed"
  | "revoked";

export interface PairingCode {
  v: 2;
  nonce: string;
  claim_url: string;
  exp: number;
}

export interface RemoteCredentials {
  pairId: string;
  deviceId: string;
  seed: string;
  userJwt: string;
  refreshToken: string;
  natsWsUrl: string;
  tokenUrl: string;
  expectedDesktopId: string;
  expectedDesktopPublicKey: string;
}

export interface RemoteSession {
  sessionId: string;
  threadId: string;
  title: string;
  mode?: "chat" | "workspace";
  workspaceId?: string;
  /** Pinned to the top of the session list (desktop `threads.pinned`). */
  pinned?: boolean;
  streaming: boolean;
  status?: string;
}

export interface RemoteWorkspace {
  id: string;
  name: string;
  path: string;
  description?: string;
}

export interface PresenceSession {
  sessionId: string;
  threadId: string;
  title: string;
  mode?: "chat" | "workspace";
  workspaceId?: string;
  /** Desktop `threads.pinned` — hoisted to the top of the session list. */
  pinned?: boolean;
  streaming: boolean;
  status?: string;
}

export interface Presence {
  online: boolean;
  /** An intentional desktop disconnect; `online: false` is authoritative. */
  disconnected?: boolean;
  /** Immediate desktop-originated unpair notice; server revocation backs it up. */
  unpaired?: boolean;
  pairId: string;
  bridgeInstanceId: string;
  lastHeartbeatTs: number;
  sessions?: PresenceSession[];
  workspaces?: RemoteWorkspace[];
}

export interface RemoteModel {
  id: string;
  label?: string;
  provider?: string;
  isDefault?: boolean;
  /** Whether the model's input modality accepts images (agent catalogue). */
  supportsImages?: boolean;
}

/** Stable agent model identifier; model ids are only unique within a provider. */
export function modelReference(model: Pick<RemoteModel, "id" | "provider">): string {
  if (!model.provider || model.id.startsWith(`${model.provider}/`)) return model.id;
  return `${model.provider}/${model.id}`;
}

export function modelProviderFromReference(modelReference: string): string | undefined {
  const separator = modelReference.indexOf("/");
  return separator > 0 ? modelReference.slice(0, separator) : undefined;
}

export interface RemoteSessionState {
  model?: string;
  thinkingLevel?: ThinkingLevel;
  /** The session's active (in-flight) run, if any — the tail of its events
   *  can be backfilled from `get_events_since` to resync on open/reconnect. */
  activeRun?: { runId?: string } | null;
}

export type ThinkingLevel = "off" | "minimal" | "low" | "medium" | "high" | "xhigh";

export interface HistoryMessage {
  role: "user" | "assistant" | "tool" | string;
  /** Omitted on the wire when null — e.g. tool-call-only assistant messages. */
  content?: string | { type?: string; text?: string }[] | null;
  run_id?: string;
}

/** Attachment chip on a user entry — mirrors the desktop `meta.attachments`. */
export interface HistoryAttachment {
  path: string;
  name: string;
  kind?: "image" | "file" | null;
  /** Local optimistic hint; durable desktop history enforces this server-side. */
  mobilePreviewUnsupported?: boolean;
}

export interface MobileAttachment {
  localUri: string;
  /** Original user-facing name; remains unchanged after JPEG conversion. */
  name: string;
  /** On-wire/on-disk name when converted bytes need a different extension. */
  transferName?: string;
  mimeType: string;
  kind: "image" | "file";
  /** Quota accounting always uses the original, pre-compression byte size. */
  originalSize: number;
  transferSize: number;
  /** True when localUri is an app-cache derivative that we own and may delete. */
  temporary?: boolean;
  mobilePreviewUnsupported?: boolean;
  uploadId?: string;
  contentHash?: string;
}

export interface DownloadInfo {
  transferId: string;
  name: string;
  mimeType: string;
  size: number;
  contentHash: string;
  previewKind: "image" | "markdown" | "text";
  chunkBytes: number;
}

/**
 * Display-shaped session entry from `get_session_entries` (the agent's JSONL,
 * same source the desktop app renders). Content is plain text; user entries
 * carry attachments on `meta`, and assistant entries may carry the model's
 * thinking and the tool calls it introduced.
 */
export interface HistoryEntry {
  id?: string;
  role: string;
  content?: string | null;
  /** Model reasoning for an assistant entry — rendered as a thinking row. */
  thinking?: string | null;
  /** Tool calls an assistant entry introduced — rendered as activity rows. */
  tool_calls?: { id?: string; function?: { name?: string; arguments?: unknown } }[] | null;
  meta?: {
    /** Canonical Agent run identity (present on new entries). */
    run_id?: string;
    attachments?: HistoryAttachment[] | null;
  } | null;
  /** Output tokens for the reply — only the final assistant entry of a run. */
  output_tokens?: number;
  /** Reply wall-clock duration in ms — paired with `output_tokens`. */
  duration_ms?: number;
  /** RFC3339 entry time; preserved across re-saves so history keeps real times. */
  timestamp?: string;
}

export interface StreamEvent {
  type: string;
  data: string;
  runId?: string;
  idx?: number;
}

export interface ApprovalPayload {
  approval_request_id: string;
  tool_name?: string;
  risk_level?: string;
  kind?: string;
  title?: string;
  summary?: string;
  /** Agent-built action object (writes/paths/command) — path is surfaced to the user. */
  action?: unknown;
}

/**
 * A tool activity row — mirrors the shared `AgentActivityItem` status surface.
 * `complete` is kept for the legacy flat-tool-row path; `status` is the
 * authoritative tool state (drives the failed style).
 */
export interface TimelineToolRow {
  name: string;
  complete: boolean;
  /** The call's display target (desktop parity): the shell command or
   *  file path, shown on the row and expandable on tap. */
  detail?: string;
  /** Authoritative tool state — "failed" renders a danger-styled row. */
  status?: "running" | "completed" | "failed";
  /** Present on a collapsed same-kind burst ("Edited 3 files") — the child
   *  calls the summary row stands for. */
  count?: number;
  children?: TimelineToolRow[];
}

/**
 * One ordered slice of an assistant reply bubble (shared `MessageSegment`
 * parity). Text/tool/thinking/compaction are kept in the chronological order
 * the agent produced them instead of being flattened. `id` is the stable
 * shared-projection segment id — React keys must use it, never `kind` (an
 * interleaved reply has many same-kind segments).
 */
export type TimelineSegment =
  | { id: string; kind: "text"; text: string }
  | { id: string; kind: "thinking"; text: string }
  | { id: string; kind: "tool"; tool: TimelineToolRow }
  | { id: string; kind: "compaction"; tokensBefore?: number };

export type TimelineItem =
  | {
      id: string;
      kind: "message";
      role: "user" | "assistant";
      text: string;
      runId?: string;
      // Live "in-flight" flag for assistant replies, mirroring the desktop app's
      // per-message `status === "streaming"`. While true the footer shows the
      // generating indicator instead of the copy button. Driven by agent_start /
      // text_chunk (set) and agent_end (cleared), so it survives a resync that
      // rebuilds the timeline from history (which carries no run indicator).
      streaming?: boolean;
      // Epoch-ms run-start anchor for the live elapsed timer. Comes from the
      // agent's `started_at_ms` when the event carries it (replay-safe), else
      // the local receipt time. Settled replies show the agent_end duration_ms
      // instead — receipt-time durations understate late-joined runs.
      startedAt?: number;
      durationMs?: number;
      /** Output tokens for the reply (real provider usage). */
      outputTokens?: number;
      attachments?: HistoryAttachment[];
      /**
       * Ordered inline slices of an assistant reply — thinking/tool rows and
       * compaction markers render *inside* the bubble in stream order (desktop
       * parity), instead of the old merged-bubble-with-rows-above layout. Falls
       * back to `text` when absent (optimistic, legacy, plain history).
       */
      segments?: TimelineSegment[];
      /** A tool row in this reply failed (shell non-zero exit / error) — the
       *  bubble gets a danger affordance. */
      failed?: boolean;
      /** The user cancelled this run (agent_end state "cancelled"). */
      stopped?: boolean;
      /** The stream ended before the model finished (agent_end reason
       *  "incomplete") — the text is a truncated prefix, not a clean answer. */
      truncated?: boolean;
    }
  | {
      id: string;
      kind: "thinking";
      text: string;
      complete: boolean;
      runId?: string;
    }
  | {
      id: string;
      kind: "tool";
      toolId: string;
      name: string;
      complete: boolean;
      runId?: string;
      /** The call's display target (desktop parity): the shell command or
       *  file path, shown on the row and expandable on tap. */
      detail?: string;
    }
  | {
      id: string;
      kind: "approval";
      payload: ApprovalPayload;
      decision?: "approved" | "rejected" | "cancelled";
      runId?: string;
    }
  | {
      id: string;
      kind: "notice";
      tone: "neutral" | "danger" | "warning";
      text: string;
      runId?: string;
    };

export interface RpcResponse<T = unknown> {
  success: boolean;
  data: T;
  error?: string;
}

export interface RemoteCommand {
  id?: string;
  type: string;
  sessionId?: string;
  message?: string;
  entryId?: string;
  mode?: string;
  runId?: string;
  sinceIdx?: number;
  offset?: number;
  limit?: number;
  modelId?: string;
  providerId?: string;
  level?: string;
  tier?: string;
  name?: string;
  transferName?: string;
  workspaceId?: string;
  /** Thread-scoped mutating commands (delete_session / set_session_pinned). */
  threadId?: string;
  pinned?: boolean;
  protocolVersion?: number;
  pairId?: string;
  deviceId?: string;
  clientPublicKey?: string;
  clientNonce?: string;
  desktopNonce?: string;
  expectedDesktopId?: string;
  expectedDesktopPublicKey?: string;
  clientSignature?: string;
  mimeType?: string;
  kind?: string;
  originalSize?: number;
  transferSize?: number;
  transferId?: string;
  filePath?: string;
  attachments?: { uploadId: string }[];
}

export interface SessionsData {
  sessions: RemoteSession[];
}

export interface ModelsData {
  models: RemoteModel[];
}

export interface WorkspacesData {
  workspaces: RemoteWorkspace[];
}

export interface HistoryData {
  messages: HistoryMessage[];
  total?: number;
  hasMore?: boolean;
  nextOffset?: number;
}

export interface EntriesData {
  entries: HistoryEntry[];
  total?: number;
  hasMore?: boolean;
  nextOffset?: number;
}

export interface PromptAck {
  sessionId: string;
  threadId: string;
  runId: string;
}
