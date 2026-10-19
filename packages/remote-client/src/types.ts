// Client-owned credentials and projection data; wire DTOs come from Protocol.
import type { HistoryAttachment, ApprovalPayload } from "@future-os/remote-protocol";
import type { ShellResult } from "@future-os/thread-projection";
export * from "@future-os/remote-protocol";

export interface RemoteCredentials {
  /** JSON-encoded v2 identity bundle, stored only in the OS credential store. */
  secureBundle?: string;
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
  shellResult?: ShellResult;
  /** The call's identity and run, on rows projected from persisted entries.
   *  A lean history page omits a shell call's arguments, so a row with no
   *  target fetches its command back by this identity when opened. */
  toolCallId?: string;
  runId?: string;
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
  | {
      id: string;
      kind: "compaction";
      /** Durable checkpoint this divider renders — see the shared model. */
      checkpointId?: string;
      tokensBefore?: number;
      /**
       * The agent's estimate of the prompt input tokens the next turn starts
       * from once the summary replaces the covered history (shared model's
       * `tokensAfter`). Absent when the agent reported no committed checkpoint.
       */
      tokensAfter?: number;
      trigger?: string;
      status?: "running" | "completed" | "failed";
      error?: string;
    };

export type TimelineItem =
  | {
      id: string;
      kind: "message";
      role: "user" | "assistant";
      text: string;
      runId?: string;
      /**
       * The canonical persisted Agent entry this user bubble came from. Forking
       * points at this identity (never rendered text), and an optimistic bubble
       * that has no entry yet simply has none.
       */
      sourceEntryId?: string;
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
      /** Prompt (input) tokens of the run (billed side); absent on legacy. */
      inputTokens?: number;
      /** Cache-read tokens of the run (informational subset of input). */
      cacheReadTokens?: number;
      attachments?: HistoryAttachment[];
      /**
       * Ordered inline slices of an assistant reply — thinking/tool rows and
       * compaction markers render *inside* the bubble in stream order (desktop
       * parity), instead of the old merged-bubble-with-rows-above layout. Falls
       * back to `text` when absent (optimistic, legacy, plain history).
       */
      segments?: TimelineSegment[];
      /** The owning run failed. A handled tool error alone is not recoverable. */
      failed?: boolean;
      /** Raw agent error of a failed run — classified into the independent
       *  status note rendered after any partial assistant content. */
      error?: string;
      /** The user cancelled this run (agent_end state "cancelled"). */
      stopped?: boolean;
      /** The stream ended before the model finished (agent_end reason
       *  "incomplete") — the text is a truncated prefix, not a clean answer. */
      truncated?: boolean;
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
