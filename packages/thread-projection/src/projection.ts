import type {
  AgentActivityItem,
  AgentMessage,
  MessageAttachment,
  MessageSegment,
} from "./model";
import type { ToolKind } from "./group";
import type { SessionEntry } from "./events";
import {
  asToolKind,
  COLLAPSIBLE_KINDS,
  dedupeByTarget,
  foldCollapsibleRuns,
  normalizeArgs,
  targetFromArgs,
} from "./group";

/** Rebuild the message's attachment chips from a user entry's meta. */
function attachmentsFromMeta(
  entry: SessionEntry,
): MessageAttachment[] | undefined {
  const items = entry.metadata?.attachments;
  if (!Array.isArray(items) || items.length === 0) return undefined;
  return items
    .filter(
      (item) =>
        item &&
        typeof item.path === "string" &&
        item.path.length > 0 &&
        typeof item.name === "string",
    )
    .map((item) => ({
      path: item.path,
      name: item.name,
      kind: item.kind ?? "file",
      thumbnail: item.thumbnail ?? null,
    }));
}

interface ExchangeAcc {
  userMessage?: AgentMessage;
  /** Canonical identity persisted on the user entry that opened this turn. */
  userRunId?: string;
  segments: MessageSegment[];
  finalText: string;
  /** Timestamp of the assistant reply (last assistant entry of the exchange wins). */
  assistantCreatedAt?: string;
  /** Key of the last assistant entry — the assistant message's stable id seed. */
  assistantEntryId?: string;
  /** Per-reply usage/timing carried on the final assistant entry. */
  outputTokens?: number;
  inputTokens?: number;
  cacheReadTokens?: number;
  durationMs?: number;
  /** Set only from an assistant entry finalized by the Agent. */
  runId?: string;
  outcome?: SessionRunOutcome;
  /**
   * Tool activities awaiting their result entry, in call order. A `tool` result
   * entry updates the oldest one's status (the agent executes and appends
   * results in order), so a failed tool doesn't reload as "completed".
   */
  pendingTools: AgentActivityItem[];
}

export interface SessionRunOutcome {
  status: "completed" | "failed" | "cancelled" | "incomplete" | string;
  error?: string;
  durationMs?: number;
}

export interface SessionTurn {
  key: string;
  user: AgentMessage;
  assistant?: AgentMessage;
  runId?: string;
  outcome?: SessionRunOutcome;
  identitySource: "canonical" | "legacy" | "conflict";
}

export type SessionProjectionNode =
  | { kind: "turn"; turn: SessionTurn }
  | { kind: "standalone"; message: AgentMessage };

/**
 * Collapse an uninterrupted burst of same-kind, completed tool activities into
 * one summary row ("编辑了 N 个文件"), matching the live/store path. A text or
 * thinking segment — or a failed tool — breaks the run.
 */
function collapseActivitySegments(
  segments: MessageSegment[],
): MessageSegment[] {
  const out: MessageSegment[] = [];
  const runs = foldCollapsibleRuns(segments, (seg) =>
    seg.kind === "activity" &&
    seg.item.status === "completed" &&
    COLLAPSIBLE_KINDS.has(seg.item.kind as ToolKind)
      ? (seg.item.kind as ToolKind)
      : null,
  );

  for (const run of runs) {
    if (!run.collapsed) {
      out.push(run.item);
      continue;
    }
    const items = run.group
      .filter(
        (seg): seg is Extract<MessageSegment, { kind: "activity" }> =>
          seg.kind === "activity",
      )
      .map((seg) => seg.item);
    const children = run.kind === "shell" ? items : dedupeByTarget(items);
    // Seed both ids from the first child tool-call id (stable across reloads)
    // so a re-projection reuses the collapsed row instead of remounting it.
    const base = items[0]?.id ?? segId();
    out.push({
      id: `seg_${base}`,
      kind: "activity",
      item: {
        id: `collapsed_${base}`,
        kind: run.kind,
        status: "completed",
        count: children.length,
        children,
      },
    });
  }

  return out;
}

let _seq = 0;
function segId(): string {
  return `ep_${Date.now()}_${++_seq}`;
}

/**
 * Stable key for deriving React ids from a JSONL entry. Entry ids are
 * assigned once by the agent and preserved across re-saves, so ids derived
 * from them survive re-projection (run settle, thread switch) — a fresh
 * segId() per projection used to remount the whole window. segId() stays as
 * the fallback for entries without one.
 */
function entryKey(entry: SessionEntry): string {
  return entry.id || segId();
}

/**
 * The agent replaces summarized history with a single user message
 * "[Context compaction: …]" (compaction/mod.rs).
 */
function isCompactionDivider(entry: SessionEntry): boolean {
  return (
    entry.kind === "compaction" ||
    (entry.role === "user" &&
      entryText(entry).startsWith("[Context compaction:"))
  );
}

/** Render a compaction marker as a divider, not as a user bubble / new exchange. */
function dividerMessage(entry: SessionEntry, now: string): AgentMessage {
  const key = entry.checkpoint?.checkpointId || entryKey(entry);
  const tokensBefore =
    typeof entry.checkpoint?.tokensBefore === "number" &&
    entry.checkpoint.tokensBefore > 0
      ? entry.checkpoint.tokensBefore
      : undefined;
  return {
    id: `m_${key}`,
    role: "assistant",
    authorKey: "author.researchCopilot",
    content: "",
    status: "complete",
    createdAt: entryTime(entry) ?? now,
    segments: [
      {
        id: `seg_${key}_compaction`,
        kind: "compaction",
        ...(tokensBefore ? { tokensBefore } : {}),
        ...(entry.checkpoint?.trigger
          ? { trigger: entry.checkpoint.trigger }
          : {}),
      },
    ],
  };
}

function newExchangeAcc(): ExchangeAcc {
  return { segments: [], finalText: "", pendingTools: [] };
}

function userMessageFromEntry(entry: SessionEntry, now: string): AgentMessage {
  return {
    id: `m_${entryKey(entry)}`,
    role: "user",
    authorKey: "author.you",
    content: entryText(entry),
    status: "complete",
    createdAt: entryTime(entry) ?? now,
    attachments: attachmentsFromMeta(entry),
    // Run identity stays off the user bubble on purpose: runId-on-assistant is
    // the convention applyRunMetadata / streamingBubbleBase use to tell
    // settled exchanges from in-flight ones — a stamped user message would
    // suppress the live streaming bubble after a mid-run reload.
  };
}

function foldRunOutcome(acc: ExchangeAcc, entry: SessionEntry) {
  const runId = entry.runId;
  if (!runId || !entry.run?.status) return;
  if (acc.userRunId && runId !== acc.userRunId) return;
  acc.outcome = {
    status: entry.run?.status,
    ...(entry.run?.error?.trim() ? { error: entry.run?.error } : {}),
    ...(typeof entry.run?.durationMs === "number" && entry.run?.durationMs >= 0
      ? { durationMs: entry.run?.durationMs }
      : {}),
  };
}

function entryText(entry: SessionEntry): string {
  return entry.blocks
    .filter((b) => b.kind === "text")
    .slice(0, entry.role === "user" ? 1 : undefined)
    .map((b) => b.text ?? "")
    .join("");
}

function entryTime(entry: SessionEntry): string | undefined {
  return Number.isFinite(entry.createdAtMs)
    ? new Date(entry.createdAtMs).toISOString()
    : undefined;
}

/** Preserve the actual reasoning/text/tool interleaving, including text after tools. */
function foldAssistantEntry(acc: ExchangeAcc, entry: SessionEntry) {
  const key = entryKey(entry);
  acc.assistantEntryId = key;
  acc.assistantCreatedAt = entryTime(entry);
  if (entry.usage?.outputTokens != null)
    acc.outputTokens = entry.usage.outputTokens;
  if (entry.usage?.inputTokens != null)
    acc.inputTokens = entry.usage.inputTokens;
  if (entry.usage?.cacheReadTokens != null)
    acc.cacheReadTokens = entry.usage.cacheReadTokens;
  if (entry.run?.durationMs != null) acc.durationMs = entry.run.durationMs;
  if (entry.runId) acc.runId = entry.runId;
  for (const [index, block] of entry.blocks.entries()) {
    const id = `seg_${key}_${index}`;
    if (block.kind === "reasoning" && block.text) {
      acc.segments.push({ id, kind: "thinking", text: block.text });
    } else if (block.kind === "text" && block.text?.trim()) {
      acc.segments.push({ id, kind: "text", text: block.text });
      acc.finalText = block.text;
    } else if (block.kind === "tool_call") {
      const kind = asToolKind(block.name ?? "");
      const target = targetFromArgs(kind, normalizeArgs(block.arguments));
      const item: AgentActivityItem = {
        id: block.toolCallId || id,
        kind,
        status: "completed",
        target,
        detail: target,
      };
      acc.segments.push({ id, kind: "activity", item });
      acc.pendingTools.push(item);
    }
  }
}

/** Results match by identity, never by whichever call happens to be first. */
function foldToolEntry(acc: ExchangeAcc | null, entry: SessionEntry) {
  if (!acc) return;
  for (const block of entry.blocks) {
    if (block.kind !== "tool_result" || !block.toolCallId) continue;
    const index = acc.pendingTools.findIndex(
      (item) => item.id === block.toolCallId,
    );
    if (index < 0) continue;
    const [item] = acc.pendingTools.splice(index, 1);
    if (block.isError && item) item.status = "failed";
  }
}

/** Finalize one user-led exchange without losing a reply-less terminal run. */
function turnFromAcc(acc: ExchangeAcc): SessionTurn | null {
  if (!acc.userMessage) return null;
  const textSegments = acc.segments.filter((s) => s.kind === "text") as {
    kind: "text";
    id: string;
    text: string;
  }[];
  // Collapse same-kind tool bursts only after statuses are final (a failing
  // tool result, processed later, must break the group).
  const segments = collapseActivitySegments(acc.segments);
  // Skip the assistant message for incomplete exchanges — the user message is
  // the last entry and the assistant reply hasn't been written to the JSONL
  // yet (the agent is still streaming). An empty completed bubble would steal
  // the runId in applyRunMetadata and block upsertStreamingPreview from
  // inserting the live preview when the user returns to this thread.
  const canonicalConflict =
    !!acc.userRunId && !!acc.runId && acc.userRunId !== acc.runId;
  const turnRunId = canonicalConflict
    ? undefined
    : (acc.userRunId ?? acc.runId);
  const outcome = canonicalConflict ? undefined : acc.outcome;
  const terminalWithoutReply =
    outcome?.status === "failed" ||
    outcome?.status === "interrupted" ||
    outcome?.status === "cancelled";
  const hasContent =
    acc.finalText ||
    textSegments.length > 0 ||
    segments.length > 0 ||
    acc.outputTokens !== undefined ||
    acc.durationMs !== undefined ||
    terminalWithoutReply;
  let assistant: AgentMessage | undefined;
  if (hasContent) {
    const stopped = outcome?.status === "cancelled";
    const failed =
      outcome?.status === "failed" || outcome?.status === "interrupted";
    assistant = {
      id: acc.assistantEntryId
        ? `m_${acc.assistantEntryId}`
        : turnRunId
          ? `${failed ? "failed" : "stopped"}_${turnRunId}`
          : segId(),
      role: "assistant",
      authorKey: "author.researchCopilot",
      content: acc.finalText || textSegments.map((s) => s.text).join("\n"),
      segments: segments.length > 0 ? segments : undefined,
      status: failed ? "failed" : "complete",
      // An aborted exchange has no assistant entry, so no recorded reply time —
      // fall back to the user message's time (a real timestamp) rather than
      // `now`, which would re-stamp the reply "just now" on every reload.
      createdAt: acc.assistantCreatedAt ?? acc.userMessage.createdAt,
      outputTokens: acc.outputTokens,
      inputTokens: acc.inputTokens,
      cacheReadTokens: acc.cacheReadTokens,
      durationMs: acc.durationMs ?? outcome?.durationMs,
      runId: canonicalConflict
        ? undefined
        : (acc.runId ?? (outcome ? turnRunId : undefined)),
      ...(stopped ? { stopped: true } : {}),
      ...(outcome?.error ? { runError: outcome.error } : {}),
    };
  }
  return {
    key: acc.userMessage.id,
    user: acc.userMessage,
    ...(assistant ? { assistant } : {}),
    ...(turnRunId ? { runId: turnRunId } : {}),
    ...(outcome ? { outcome } : {}),
    identitySource: canonicalConflict
      ? "conflict"
      : turnRunId
        ? "canonical"
        : "legacy",
  };
}

/**
 * Convert raw agent session entries into AgentMessage[] for the GUI pipeline.
 * Each user→assistant exchange yields 1 user + 1 assistant message with
 * segments for thinking, tool activity, and text.
 *
 * Grouping is positional: a user entry always opens a new exchange (each run
 * has exactly one user message, so journal order is conversation order).
 */
export function entriesToTurns(
  entries: SessionEntry[],
): SessionProjectionNode[] {
  const nodes: SessionProjectionNode[] = [];
  const now = new Date().toISOString();

  const flush = (acc: ExchangeAcc) => {
    const turn = turnFromAcc(acc);
    if (turn) nodes.push({ kind: "turn", turn });
  };

  let acc: ExchangeAcc | null = null;
  for (const entry of entries) {
    if (isCompactionDivider(entry)) {
      if (acc) {
        flush(acc);
        acc = null;
      }
      nodes.push({ kind: "standalone", message: dividerMessage(entry, now) });
      continue;
    }
    if (entry.role === "user") {
      if (acc) {
        flush(acc);
        acc = null;
      }
      acc = newExchangeAcc();
      acc.userMessage = userMessageFromEntry(entry, now);
      if (typeof entry.runId === "string" && entry.runId)
        acc.userRunId = entry.runId;
      foldRunOutcome(acc, entry);
    } else if (entry.role === "assistant") {
      if (!acc) acc = newExchangeAcc();
      foldAssistantEntry(acc, entry);
      foldRunOutcome(acc, entry);
    } else if (entry.role === "tool") {
      foldToolEntry(acc, entry);
      if (acc) foldRunOutcome(acc, entry);
    }
  }
  if (acc) flush(acc);
  return nodes;
}

export function turnsToMessages(
  nodes: SessionProjectionNode[],
): AgentMessage[] {
  return nodes.flatMap((node) =>
    node.kind === "standalone"
      ? [node.message]
      : [node.turn.user, ...(node.turn.assistant ? [node.turn.assistant] : [])],
  );
}

export function entriesToMessages(entries: SessionEntry[]): AgentMessage[] {
  return turnsToMessages(entriesToTurns(entries));
}
