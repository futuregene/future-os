/**
 * Wire types the projection reads: the raw agent event log and the session
 * entry records. Pure data, mirroring the shapes the desktop agent bridge and
 * the remote protocol send across the wire. Field names follow the gRPC/RPC
 * convention (snake_case) because both ends serialize against them.
 */

/** One raw event from a run's event log (desktop StoredRunEvent / remote stream). */
export interface RunEvent {
  id: string;
  runId: string;
  eventType: string;
  payload?: string | null;
  sequence: number;
  createdAt: number;
}

/** Ordered blocks returned by the existing history RPC. */
export interface MessageBlock {
  kind: string;
  text?: string;
  toolCallId?: string;
  name?: string;
  arguments?: unknown;
  isError?: boolean;
  imageUrl?: string;
  providerMetadata?: unknown;
  data?: unknown;
}

export interface SessionEntry {
  id: string;
  kind: string;
  role: "user" | "assistant" | "tool" | "system";
  createdAtMs: number;
  runId?: string | null;
  blocks: MessageBlock[];
  metadata?: {
    attachments?: Array<{
      path: string;
      name: string;
      kind?: "image" | "file" | null;
      thumbnail?: string | null;
    }>;
    [key: string]: unknown;
  } | null;
  usage?: {
    inputTokens?: number | null;
    outputTokens?: number | null;
    cacheReadTokens?: number | null;
    cacheWriteTokens?: number | null;
  } | null;
  run?: {
    status?: string | null;
    error?: string | null;
    durationMs?: number | null;
  } | null;
  session?: Record<string, unknown> | null;
  checkpoint?: {
    schemaVersion?: number;
    checkpointId?: string;
    cutoffEntryId?: string;
    tokensBefore?: number;
    tokensAfter?: number;
    trigger?: string;
    phase?: string;
    algorithmVersion?: string;
    summary?: unknown;
  } | null;
}
