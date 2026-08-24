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

/** Raw entry from agent get_session_entries RPC. */
export interface SessionEntry {
  id: string;
  entry_type?: string;
  role: "user" | "assistant" | "tool" | "system";
  content: string;
  name?: string;
  tool_args?: string;
  thinking?: string;
  tool_calls?: Array<{ id: string; function: { name: string; arguments: unknown } }>;
  /** RFC3339 entry time; preserved across re-saves so history keeps real times. */
  timestamp?: string;
  /** Output tokens for the reply — only the final assistant entry of a run. */
  output_tokens?: number;
  /** Run wall-clock duration in ms — paired with `output_tokens`. */
  duration_ms?: number;
  /** Prompt (input) tokens of the run — the session's cumulative tokens_in
   * delta for this run. Absent on legacy sessions. */
  input_tokens?: number;
  /** Cache-read tokens of the run (informational subset of input_tokens). */
  cache_read_tokens?: number;
  /** Structured per-entry metadata; user entries carry attached files here. */
  meta?: {
    /** Canonical Agent run identity (new entries; absent in legacy JSONL). */
    run_id?: string;
    attachments?: Array<{
      path: string;
      kind?: "image" | "file" | null;
      name: string;
      thumbnail?: string | null;
    }>;
  };
  /** Durable context-checkpoint payload for entry_type=compaction. */
  checkpoint?: {
    schema_version?: number;
    checkpoint_id?: string;
    cutoff_entry_id?: string;
    tokens_before?: number;
    tokens_after?: number;
    trigger?: string;
    algorithm_version?: string;
    summary?: unknown;
  };
}
