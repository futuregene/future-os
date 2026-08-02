import type { AgentActivityItem, AgentMessage, MessageAttachment, MessageSegment } from "./agentThreadTypes";
import type { ToolKind } from "./toolActivityModel";
import { isSoftExit, nonZeroExitCode } from "./agentActivity";
import {
  asToolKind,
  COLLAPSIBLE_KINDS,
  dedupeByTarget,
  foldCollapsibleRuns,
  normalizeArgs,
  targetFromArgs,
} from "./toolActivityModel";

/** Raw entry from agent get_session_entries RPC. */
export interface SessionEntry {
  id: string;
  role: "user" | "assistant" | "tool";
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
  /** Structured per-entry metadata; user entries carry attached files here. */
  meta?: {
    /** Canonical Agent run identity (new entries; absent in legacy JSONL). */
    run_id?: string;
    /** Conversation-turn identity (new entries; absent in legacy JSONL). */
    turn_id?: string;
    attachments?: Array<{
      path: string;
      kind?: "image" | "file" | null;
      name: string;
      thumbnail?: string | null;
    }>;
  };
}

/** Rebuild the message's attachment chips from a user entry's meta. */
function attachmentsFromMeta(entry: SessionEntry): MessageAttachment[] | undefined {
  const items = entry.meta?.attachments;
  if (!Array.isArray(items) || items.length === 0)
    return undefined;
  return items.map(item => ({
    path: item.path,
    name: item.name,
    kind: item.kind ?? "file",
    thumbnail: item.thumbnail ?? null,
  }));
}

interface TurnAcc {
  userMessage?: AgentMessage;
  segments: MessageSegment[];
  finalText: string;
  /** Timestamp of the assistant reply (last assistant entry of the turn wins). */
  assistantCreatedAt?: string;
  /** Per-reply usage/timing carried on the final assistant entry. */
  outputTokens?: number;
  durationMs?: number;
  /** Set only from an assistant entry finalized by the Agent. */
  runId?: string;
  /** The turn this accumulator groups, when the journal carries turn ids. */
  turnId?: string;
  /**
   * Tool activities awaiting their result entry, in call order. A `tool` result
   * entry updates the oldest one's status (the agent executes and appends
   * results in order), so a failed tool doesn't reload as "completed".
   */
  pendingTools: AgentActivityItem[];
}

/**
 * Whether a tool result's content marks a failure: the agent prefixes a tool
 * error with "Error: ", and a shell non-zero exit puts an "[exit: N]" footer at the end of the
 * output (with the bare grep/diff/test exit-1 soft-fail exemption).
 */
function toolResultFailed(content: string, command: string | undefined): boolean {
  if (!content)
    return false;
  if (content.startsWith("Error:"))
    return true;
  const code = nonZeroExitCode(content);
  if (code === null)
    return false;
  return !isSoftExit(code, command);
}

/**
 * Collapse an uninterrupted burst of same-kind, completed tool activities into
 * one summary row ("编辑了 N 个文件"), matching the live/store path. A text or
 * thinking segment — or a failed tool — breaks the run.
 */
function collapseActivitySegments(segments: MessageSegment[]): MessageSegment[] {
  const out: MessageSegment[] = [];
  const runs = foldCollapsibleRuns(segments, seg =>
    seg.kind === "activity" && seg.item.status === "completed" && COLLAPSIBLE_KINDS.has(seg.item.kind as ToolKind)
      ? (seg.item.kind as ToolKind)
      : null);

  for (const run of runs) {
    if (!run.collapsed) {
      out.push(run.item);
      continue;
    }
    const items = run.group
      .filter((seg): seg is Extract<MessageSegment, { kind: "activity" }> => seg.kind === "activity")
      .map(seg => seg.item);
    const children = run.kind === "shell" ? items : dedupeByTarget(items);
    out.push({
      id: segId(),
      kind: "activity",
      item: { id: segId(), kind: run.kind, status: "completed", count: children.length, children },
    });
  }

  return out;
}

let _seq = 0;
function segId(): string {
  return `ep_${Date.now()}_${++_seq}`;
}

/**
 * The agent replaces summarized history with a single user message
 * "[Context compaction: …]" (compaction/mod.rs).
 */
function isCompactionDivider(entry: SessionEntry): boolean {
  return entry.role === "user" && entry.content.startsWith("[Context compaction:");
}

/** Render a compaction marker as a divider, not as a user bubble / new turn. */
function dividerMessage(entry: SessionEntry, now: string): AgentMessage {
  return {
    id: segId(),
    role: "assistant",
    authorKey: "author.researchCopilot",
    content: "",
    status: "complete",
    createdAt: entry.timestamp ?? now,
    segments: [{ id: segId(), kind: "compaction" }],
  };
}

function newTurnAcc(): TurnAcc {
  return { segments: [], finalText: "", pendingTools: [] };
}

function userMessageFromEntry(entry: SessionEntry, now: string): AgentMessage {
  return {
    id: segId(),
    role: "user",
    authorKey: "author.you",
    content: entry.content,
    status: "complete",
    createdAt: entry.timestamp ?? now,
    attachments: attachmentsFromMeta(entry),
    // Run identity stays off the user bubble on purpose: runId-on-assistant is
    // the convention applyRunMetadata / streamingBubbleBase use to tell
    // settled turns from in-flight ones — a stamped user message would
    // suppress the live streaming bubble after a mid-run reload. The turn id
    // carries the message's journal identity instead.
    turnId: entry.meta?.turn_id ?? null,
  };
}

/** Fold one assistant entry into a turn's accumulator. */
function foldAssistantEntry(acc: TurnAcc, entry: SessionEntry) {
  // Last assistant entry of the turn carries the reply's time + usage.
  if (entry.timestamp)
    acc.assistantCreatedAt = entry.timestamp;
  if (typeof entry.output_tokens === "number")
    acc.outputTokens = entry.output_tokens;
  if (typeof entry.duration_ms === "number")
    acc.durationMs = entry.duration_ms;
  if (typeof entry.meta?.run_id === "string" && entry.meta.run_id)
    acc.runId = entry.meta.run_id;
  if (entry.thinking) {
    acc.segments.push({ id: segId(), kind: "thinking", text: entry.thinking });
  }
  // Text (any preamble) comes before the tool calls it introduces — that's
  // the order the model emits within a message, and the order the live path
  // shows. Pushing tools first put "Read config.toml" above "Let me check
  // the config".
  if (entry.content?.trim()) {
    acc.segments.push({ id: segId(), kind: "text", text: entry.content });
    acc.finalText = entry.content;
  }
  if (entry.tool_calls) {
    for (const tc of entry.tool_calls) {
      const kind = asToolKind(tc.function.name);
      const target = targetFromArgs(kind, normalizeArgs(tc.function.arguments));
      const item: AgentActivityItem = {
        // Use the LLM's tool call id (call_00_xxx) so it matches the
        // stored tool call records in the runs panel.
        id: tc.id || segId(),
        kind,
        status: "completed",
        target,
        // The path/command, not the raw args blob — matches the live path and
        // keeps a write's hover from being its entire file content.
        detail: target,
      };
      acc.segments.push({ id: segId(), kind: "activity", item });
      acc.pendingTools.push(item);
    }
  }
}

/**
 * A `tool` result entry doesn't get its own row (the assistant's
 * `tool_calls` already produced one — rendering it too duplicated the row
 * as a blank activity). Use it only to mark that call failed, matching the
 * tool_calls in order (the agent executes and appends results in order).
 */
function foldToolEntry(acc: TurnAcc | null, entry: SessionEntry) {
  const item = acc?.pendingTools.shift();
  if (item) {
    const command = item.kind === "shell" ? item.target : undefined;
    if (toolResultFailed(entry.content, command))
      item.status = "failed";
  }
}

/** Emit a completed turn as 1 user message + (when non-empty) 1 assistant. */
function flushAcc(messages: AgentMessage[], acc: TurnAcc) {
  if (!acc.userMessage)
    return;
  messages.push(acc.userMessage);
  const textSegments = acc.segments.filter(s => s.kind === "text") as { kind: "text"; id: string; text: string }[];
  // Collapse same-kind tool bursts only after statuses are final (a failing
  // tool result, processed later, must break the group).
  const segments = collapseActivitySegments(acc.segments);
  // Skip assistant message for incomplete turns — the user message is the last
  // entry and the assistant reply hasn't been written to the JSONL yet (the
  // agent is still streaming). An empty completed bubble would steal the runId
  // in applyRunMetadata and block upsertStreamingPreview from inserting the
  // live preview when the user returns to this thread.
  const hasContent = acc.finalText
    || textSegments.length > 0
    || segments.length > 0
    || acc.outputTokens !== undefined
    || acc.durationMs !== undefined;
  if (hasContent) {
    messages.push({
      id: segId(),
      role: "assistant",
      authorKey: "author.researchCopilot",
      content: acc.finalText || textSegments.map(s => s.text).join("\n"),
      segments: segments.length > 0 ? segments : undefined,
      status: "complete",
      // An aborted turn has no assistant entry, so no recorded reply time — fall
      // back to the turn's user time (a real timestamp) rather than `now`, which
      // would re-stamp the reply "just now" on every reload.
      createdAt: acc.assistantCreatedAt ?? acc.userMessage.createdAt,
      outputTokens: acc.outputTokens,
      durationMs: acc.durationMs,
      runId: acc.runId,
      turnId: acc.turnId ?? null,
    });
  }
}

/**
 * Convert raw agent session entries into AgentMessage[] for the GUI pipeline.
 * Each user→assistant turn yields 1 user + 1 assistant message with segments
 * for thinking, tool activity, and text.
 *
 * Two grouping strategies: when the journal carries turn ids on every turn
 * entry (current agents), entries are grouped by `turn_id`, so an in-run
 * follow-up/steer whose journal order differs from conversation order still
 * attributes content to the right turn. Legacy journals (no turn ids) group
 * positionally — a user entry always opens a turn.
 */
export function entriesToMessages(entries: SessionEntry[]): AgentMessage[] {
  const messages: AgentMessage[] = [];
  const now = new Date().toISOString();
  const keyed = entries.length > 0 && entries
    .filter(e => !isCompactionDivider(e) && e.role !== "tool")
    .every(e => typeof e.meta?.turn_id === "string" && e.meta.turn_id);

  if (keyed) {
    const accs = new Map<string, TurnAcc>();
    const ordered: TurnAcc[] = [];
    let open: TurnAcc | null = null;
    const accFor = (turnId: string | undefined): TurnAcc => {
      if (turnId === undefined) {
        // Only reachable for a stray tool entry — attach to the open turn.
        if (!open) {
          open = newTurnAcc();
          ordered.push(open);
        }
        return open;
      }
      let acc = accs.get(turnId);
      if (!acc) {
        acc = newTurnAcc();
        acc.turnId = turnId;
        accs.set(turnId, acc);
        ordered.push(acc);
      }
      open = acc;
      return acc;
    };
    for (const entry of entries) {
      if (isCompactionDivider(entry)) {
        messages.push(dividerMessage(entry, now));
        continue;
      }
      if (entry.role === "user")
        accFor(entry.meta?.turn_id).userMessage = userMessageFromEntry(entry, now);
      else if (entry.role === "assistant")
        foldAssistantEntry(accFor(entry.meta?.turn_id), entry);
      else if (entry.role === "tool")
        foldToolEntry(accFor(entry.meta?.turn_id), entry);
    }
    for (const acc of ordered)
      flushAcc(messages, acc);
    return messages;
  }

  // Positional grouping (legacy journals, mixed files, compaction rewrites).
  let acc: TurnAcc | null = null;
  for (const entry of entries) {
    if (isCompactionDivider(entry)) {
      if (acc) {
        flushAcc(messages, acc);
        acc = null;
      }
      messages.push(dividerMessage(entry, now));
      continue;
    }
    if (entry.role === "user") {
      if (acc) {
        flushAcc(messages, acc);
        acc = null;
      }
      acc = newTurnAcc();
      acc.turnId = entry.meta?.turn_id;
      acc.userMessage = userMessageFromEntry(entry, now);
    }
    else if (entry.role === "assistant") {
      if (!acc)
        acc = newTurnAcc();
      foldAssistantEntry(acc, entry);
    }
    else if (entry.role === "tool") {
      foldToolEntry(acc, entry);
    }
  }
  if (acc)
    flushAcc(messages, acc);
  return messages;
}
