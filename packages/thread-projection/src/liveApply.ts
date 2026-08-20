import type { RunEvent } from "./events";
import type { AgentActivityItem, AgentActivityKind, MessageSegment } from "./model";
import { isRecord, pathBasename, singleLine } from "./utils";
import {
  COLLAPSIBLE_KINDS,
  dedupeByTarget,
  foldCollapsibleRuns,
  isToolKind,
  normalizeArgs,
  targetFromArgs,
} from "./group";

export interface AssistantRunProjection {
  activityItems: AgentActivityItem[];
  content: string;
  /** Text and activity in chronological order — drives inline rendering. */
  segments: MessageSegment[];
  /**
   * Tokens this reply actually generated: summed completion tokens across
   * every LLM call in the run (0 when the provider returned no usage).
   */
  outputTokens: number;
  /**
   * The model's reasoning/thinking text for this exchange (empty when none). Blocks
   * separated by blank lines; rendered only when the "show thinking" setting is
   * on. Extracted from `thinking_delta` events — always captured, gated at render.
   */
  thinking: string;
  /**
   * The model is mid-reasoning with nothing else to show yet (no answer text, no
   * tool work). Drives the footer "thinking…" hint while the show-thinking
   * setting is off; not rendered as a top-of-message line.
   */
  thinkingActive: boolean;
  /**
   * The stream ended before the model finished: the agent's `agent_end` carried
   * reason "incomplete", so `content` is a truncated prefix. Mirrors desktop's
   * stream `complete` flag (responseInterrupted) — both ends must treat a
   * cut-off reply as failed, never as a clean completion.
   */
  truncated: boolean;
  /**
   * The user cancelled the run: the agent's `agent_end` carried `state:
   * "cancelled"` (distinct from a truncated stream, which is `reason:
   * "incomplete"`). The reply is a partial prefix and must not render as a
   * clean completion, but it is not a failure either.
   */
  stopped: boolean;
}

/**
 * Ordered placeholder: a text run or a reference to a tool by id. The tool's
 * latest state lives in the `toolActivities` map; the slot only fixes position.
 */
type Slot
  = | { type: "text"; text: string }
    | { type: "thinking"; text: string }
    | { type: "tool"; id: string }
    | { type: "compaction"; tokensBefore: number };

interface ToolActivity {
  id: string;
  kind: Exclude<AgentActivityKind, "thinking">;
  status: AgentActivityItem["status"];
  target?: string;
  detail?: string;
  argsText?: string;
  order: number;
}

/**
 * A stateful, incremental projector over a run's event log. The live-preview
 * poll fetches only the events it hasn't seen (`list_run_events_since`) and
 * ingests them here, so each tick costs O(new events) for parsing plus
 * O(slots) for the snapshot — instead of re-parsing and re-projecting the
 * whole log on every update (O(n) per update, O(n²) over a run).
 */
export interface RunProjector {
  /** Highest event sequence ingested so far (-1 when none). */
  readonly lastSequence: number;
  /**
   * Ingest a batch of events and return the current projection snapshot.
   * Batches are sorted by sequence internally; events already ingested
   * (sequence <= lastSequence) are skipped, so overlapping batches are safe.
   */
  ingest: (events: RunEvent[]) => AssistantRunProjection;
}

export function createRunProjector(options?: { preferEndTokens?: boolean }): RunProjector {
  const toolActivities = new Map<string, ToolActivity>();
  // Ordered timeline of the exchange. Text accumulates into the open text slot;
  // each tool call pins a slot at the point it started.
  const slots: Slot[] = [];
  const slottedToolIds = new Set<string>();
  let openText: Extract<Slot, { type: "text" }> | null = null;
  // The currently-open reasoning block; thinking_delta text appends here so each
  // block keeps its own position in the timeline (interleaved with text/tools).
  let openThinking: Extract<Slot, { type: "thinking" }> | null = null;
  let content = "";
  let thinking = false;
  let sawVisibleWork = false;
  let activeToolCallId: string | null = null;
  // Output tokens: prefer summing per-call `usage` events; fall back to the
  // `agent_end` total only when no per-call usage was streamed.
  let usageOutputSum = 0;
  let sawUsageEvent = false;
  let agentEndOutput = 0;
  // Set when the agent's `agent_end` carries reason "incomplete" — the stream
  // was truncated before a genuine finish (mirrors the Rust bridge's
  // `agent_end_incomplete`).
  let truncated = false;
  // Set when the agent's `agent_end` carried `state: "cancelled"` — the user
  // stopped the run (distinct from a truncated stream).
  let stopped = false;
  let lastSequence = -1;

  function processEvent(event: RunEvent) {
    const payload = parseEventPayload(event.payload);

    if (event.eventType === "usage") {
      usageOutputSum += usageOutputTokens(payload);
      sawUsageEvent = true;
      return;
    }

    if (event.eventType === "agent_end") {
      agentEndOutput = usageOutputTokens(payload);
      if (isRecord(payload)) {
        if (payload.reason === "incomplete")
          truncated = true;
        // The user cancelled the run (distinct from an incomplete stream).
        if (payload.state === "cancelled")
          stopped = true;
      }
      return;
    }

    if (event.eventType === "text_chunk") {
      const text = textFromPayload(payload);
      content += text;
      // Visible text ends any open reasoning block; later thinking opens a new one.
      openThinking = null;
      if (!openText) {
        openText = { type: "text", text: "" };
        slots.push(openText);
      }
      openText.text += text;
      if (text.trim()) {
        sawVisibleWork = true;
      }
      return;
    }

    if (event.eventType === "thinking_start") {
      thinking = true;
      // Open a new reasoning slot at this point in the timeline. A text run ends
      // here so the block sits between the surrounding text/tools, not hoisted.
      openThinking = { type: "thinking", text: "" };
      slots.push(openThinking);
      openText = null;
      return;
    }

    if (event.eventType === "thinking_delta") {
      const text = textFromPayload(payload);
      if (text) {
        // Tolerate a delta without a preceding start by opening a block lazily.
        if (!openThinking) {
          openThinking = { type: "thinking", text: "" };
          slots.push(openThinking);
          openText = null;
        }
        openThinking.text += text;
      }
      return;
    }

    if (event.eventType === "thinking_end") {
      thinking = false;
      openThinking = null;
      return;
    }

    // Context compaction ran this exchange (usually at the top, before any text).
    // Pin a marker at this point so the reply shows where history was summarized.
    // `compaction_end` carries the pre-compaction token count; the retry-path
    // variant reports 0. An aborted compaction changed nothing — skip it.
    if (event.eventType === "compaction_end") {
      if (!isRecord(payload) || payload.aborted !== true) {
        slots.push({
          type: "compaction",
          tokensBefore: numberFromPayload(payload, ["tokens_before", "tokensBefore"]),
        });
        openText = null;
        openThinking = null;
      }
      return;
    }

    if (event.eventType === "toolcall_start" || event.eventType === "tool_start") {
      const tool = toolFromPayload(payload, event.sequence);
      if (tool) {
        activeToolCallId = tool.id;
        toolActivities.set(tool.id, {
          ...toolActivities.get(tool.id),
          ...tool,
          status: "running",
        });
        if (!slottedToolIds.has(tool.id)) {
          slots.push({ type: "tool", id: tool.id });
          slottedToolIds.add(tool.id);
        }
        // A tool call ends the current text/thinking run; later output opens a
        // fresh slot so ordering is preserved.
        openText = null;
        openThinking = null;
        sawVisibleWork = true;
      }
      return;
    }

    if (event.eventType === "tool_delta" || event.eventType === "toolcall_delta") {
      if (!activeToolCallId)
        return;

      const existing = toolActivities.get(activeToolCallId);
      if (!existing)
        return;

      // Some providers periodically send the entire accumulated argument
      // string instead of a fragment. The agent preserves that distinction in
      // the wire payload, so replaying such a delta must replace rather than
      // duplicate the prior prefix.
      const deltaText = textFromPayload(payload);
      const nextArgsText = isRecord(payload) && payload.snapshot === true
        ? deltaText
        : `${existing.argsText ?? ""}${deltaText}`;
      const target = targetFromToolArgs(existing.kind, nextArgsText);
      toolActivities.set(activeToolCallId, {
        ...existing,
        argsText: nextArgsText,
        ...(target
          ? {
              detail: target,
              target: singleLine(target),
            }
          : {}),
      });
      return;
    }

    if (event.eventType === "tool_end" || event.eventType === "tool_result") {
      const tool = toolFromPayload(payload, event.sequence);
      const explicitId = explicitToolId(payload);
      // A result may omit the tool name (some serializers drop it); resolve it
      // by the explicit id against an already-tracked tool so the row still
      // completes instead of reading "running" forever.
      const toolId = explicitId
        ?? (tool ? latestRunningToolId(toolActivities, tool.kind) : undefined)
        ?? tool?.id;
      if (!toolId)
        return;
      const existing = toolActivities.get(toolId);
      if (!tool) {
        if (!existing)
          return;
        if (activeToolCallId === toolId)
          activeToolCallId = null;
        toolActivities.set(toolId, {
          ...existing,
          status: hasToolError(payload, existing.detail) ? "failed" : "completed",
        });
        sawVisibleWork = true;
        return;
      }

      if (activeToolCallId === toolId)
        activeToolCallId = null;
      toolActivities.set(toolId, {
        ...existing,
        ...tool,
        id: toolId,
        status: hasToolError(payload, existing?.detail ?? tool.detail) ? "failed" : "completed",
        order: existing?.order ?? tool.order,
        // The end event carries the result, not the args, so `tool.target` is
        // usually undefined here — keep the target/detail captured while the
        // args streamed instead of letting the spread clobber them.
        target: tool.target ?? existing?.target,
        detail: tool.detail ?? existing?.detail,
      });
      // A result without a preceding start still deserves a slot.
      if (!slottedToolIds.has(toolId)) {
        slots.push({ type: "tool", id: toolId });
        slottedToolIds.add(toolId);
      }
      sawVisibleWork = true;
    }
  }

  function snapshot(): AssistantRunProjection {
    const segments = buildSegments(slots, toolActivities);

    // Flat activity list kept for back-compat (legacy render path / callers).
    const items = collapseToolActivities([...toolActivities.values()].sort((a, b) => a.order - b.order));
    // Mid-reasoning with nothing visible yet. Reported as a flag (consumed by the
    // footer hint) rather than injected as a top-of-message "thinking" activity.
    const thinkingActive = Boolean(thinking) && !content.trim() && !sawVisibleWork;

    // Concatenated reasoning (blocks joined by blank lines) — the inline segments
    // carry the ordered form; this is the whole-exchange text for any non-inline use.
    const thinkingText = slots
      .filter((slot): slot is Extract<Slot, { type: "thinking" }> => slot.type === "thinking")
      .map(slot => slot.text.trim())
      .filter(Boolean)
      .join("\n\n");

    return {
      activityItems: items,
      content,
      segments,
      // Desktop prefers summing per-call usage. A late-joining mobile client
      // only saw the run's tail, so its sum understates the total — it prefers
      // the authoritative agent_end total instead (opt-in via the option).
      outputTokens: options?.preferEndTokens
        ? (agentEndOutput || usageOutputSum)
        : (sawUsageEvent ? usageOutputSum : agentEndOutput),
      thinking: thinkingText,
      thinkingActive,
      truncated,
      stopped,
    };
  }

  return {
    get lastSequence() {
      return lastSequence;
    },
    ingest(events) {
      const sorted = [...events].sort((a, b) => a.sequence - b.sequence);
      for (const event of sorted) {
        // Skip already-ingested events (overlapping batches), but compare
        // against the pre-batch watermark so two events sharing one sequence
        // within a single batch are both processed.
        if (event.sequence <= lastSequence)
          continue;
        processEvent(event);
      }
      if (sorted.length > 0) {
        lastSequence = Math.max(lastSequence, sorted[sorted.length - 1]!.sequence);
      }
      return snapshot();
    },
  };
}

/** One-shot full projection (settle path, tests): project the entire log. */
export function buildAssistantRunProjection(events: RunEvent[]): AssistantRunProjection {
  return createRunProjector().ingest(events);
}

function numberFromPayload(payload: unknown, keys: string[]): number {
  if (!isRecord(payload))
    return 0;
  for (const key of keys) {
    const value = payload[key];
    if (typeof value === "number" && Number.isFinite(value))
      return value;
  }
  return 0;
}

/**
 * Output (completion) tokens from a usage-bearing event. The gRPC StreamEvent
 * nests the raw `Usage` under a `usage` key (`{type,usage:{completion_tokens}}`),
 * matching how the TUI reads it; we also tolerate a flat shape just in case.
 */
function usageOutputTokens(payload: unknown): number {
  if (!isRecord(payload))
    return 0;
  const usage = isRecord(payload.usage) ? payload.usage : payload;
  return numberFromPayload(usage, ["completion_tokens", "output_tokens"]);
}

/**
 * Walk the ordered slots into renderable segments. Adjacent tool slots (ignoring
 * whitespace-only text between them) are grouped with the same collapse rules as
 * the flat list, so a burst of edits still reads as "Edited N files" — but real
 * prose between tools keeps them as separate inline lines.
 */
function buildSegments(
  slots: Slot[],
  toolActivities: Map<string, ToolActivity>,
): MessageSegment[] {
  const segments: MessageSegment[] = [];
  let index = 0;

  while (index < slots.length) {
    const slot = slots[index];
    if (!slot)
      break;

    if (slot.type === "text") {
      if (slot.text.trim()) {
        segments.push({ kind: "text", id: `text_${index}`, text: slot.text });
      }
      index += 1;
      continue;
    }

    if (slot.type === "thinking") {
      if (slot.text.trim()) {
        segments.push({ kind: "thinking", id: `thinking_${index}`, text: slot.text });
      }
      index += 1;
      continue;
    }

    if (slot.type === "compaction") {
      segments.push({
        kind: "compaction",
        id: `compaction_${index}`,
        tokensBefore: slot.tokensBefore > 0 ? slot.tokensBefore : undefined,
      });
      index += 1;
      continue;
    }

    // Gather a run of adjacent tool slots, hopping over whitespace-only text.
    const run: ToolActivity[] = [];
    let cursor = index;
    while (cursor < slots.length) {
      const current = slots[cursor];
      if (!current)
        break;
      if (current.type === "tool") {
        const tool = toolActivities.get(current.id);
        if (tool)
          run.push(tool);
        cursor += 1;
        continue;
      }
      // A compaction marker breaks the tool run — it renders as its own divider.
      if (current.type === "compaction")
        break;
      if (!current.text.trim()) {
        cursor += 1;
        continue;
      }
      break;
    }

    for (const item of collapseToolActivities(run)) {
      segments.push({ kind: "activity", id: item.id, item });
    }
    index = cursor;
  }

  return segments;
}

function latestRunningToolId(
  toolActivities: Map<string, ToolActivity>,
  kind: ToolActivity["kind"],
) {
  const latestRunning = [...toolActivities.values()]
    .filter(item => item.kind === kind && item.status === "running")
    .sort((a, b) => b.order - a.order);
  return latestRunning[0]?.id;
}

function collapseToolActivities(tools: ToolActivity[]): AgentActivityItem[] {
  const items: AgentActivityItem[] = [];
  const runs = foldCollapsibleRuns(tools, tool =>
    tool.status === "completed" && COLLAPSIBLE_KINDS.has(tool.kind) ? tool.kind : null);

  for (const run of runs) {
    if (!run.collapsed) {
      items.push(toActivityItem(run.item));
      continue;
    }
    // Shell counts every call (each command stands on its own); file tools count
    // distinct files so "Edited 3 files" matches the expanded list even when the
    // same file was touched several times in the burst. The kept children back
    // both the collapsed preview and the expanded rows.
    const childTools = run.kind === "shell" ? run.group : dedupeByTarget(run.group);
    items.push({
      id: `${run.kind}_${run.group[0]!.order}_group`,
      kind: run.kind,
      status: "completed",
      count: childTools.length,
      children: childTools.map(toActivityItem),
    });
  }

  return items;
}

function toActivityItem(tool: ToolActivity): AgentActivityItem {
  return {
    id: tool.id,
    kind: tool.kind,
    status: tool.status,
    target: tool.target,
    detail: tool.detail,
  };
}

function toolFromPayload(payload: unknown, sequence: number): ToolActivity | null {
  if (!isRecord(payload))
    return null;

  const name = stringValue(payload.tool_name)
    ?? stringValue(payload.toolName)
    ?? stringValue(payload.name);
  if (!name || !isToolKind(name))
    return null;

  const args = normalizeArgs(payload.tool_args ?? payload.toolArgs ?? payload.arguments);
  const target = targetFromArgs(name, args);

  return {
    id: explicitToolId(payload) ?? `${name}_${sequence}`,
    kind: name,
    status: "running",
    target: target ? singleLine(target) : undefined,
    detail: target,
    order: sequence,
  };
}

function targetFromToolArgs(
  kind: Exclude<AgentActivityKind, "thinking">,
  argsText: string,
) {
  const parsed = targetFromArgs(kind, normalizeArgs(argsText));
  if (parsed)
    return parsed;

  // Mid-stream the args JSON is still incomplete (e.g. write's `content` is
  // half-emitted), so `JSON.parse` fails and we'd show no target until the tool
  // finishes. The path/command sits at the front of the object and is usually
  // complete already — pull it straight out of the partial text.
  return partialTargetFromArgsText(kind, argsText);
}

function partialTargetFromArgsText(
  kind: Exclude<AgentActivityKind, "thinking">,
  argsText: string,
) {
  const keys = kind === "shell" ? ["command"] : ["path", "file_path", "filePath"];
  for (const key of keys) {
    const value = matchJsonStringField(argsText, key);
    if (value)
      return value;
  }
  return undefined;
}

function matchJsonStringField(text: string, key: string) {
  // Match `"key": "<complete quoted value>"` — only fires once the value's
  // closing quote has streamed in, so we never surface a truncated path.
  const match = new RegExp(`"${key}"\\s*:\\s*("(?:[^"\\\\]|\\\\.)*")`).exec(text);
  if (!match?.[1])
    return undefined;
  try {
    const parsed = JSON.parse(match[1]) as unknown;
    return typeof parsed === "string" ? parsed : undefined;
  }
  catch {
    return undefined;
  }
}

function explicitToolId(payload: unknown) {
  if (!isRecord(payload))
    return undefined;

  return stringValue(payload.tool_id)
    ?? stringValue(payload.toolID)
    ?? stringValue(payload.tool_call_id);
}

function parseEventPayload(payload?: string | null): unknown {
  if (!payload)
    return null;

  try {
    return JSON.parse(payload) as unknown;
  }
  catch {
    return null;
  }
}

function textFromPayload(payload: unknown) {
  if (!isRecord(payload))
    return "";

  return stringValue(payload.text)
    ?? stringValue(payload.delta)
    ?? stringValue(payload.content)
    ?? "";
}

// Bare grep/diff/cmp/test exiting 1 is a normal "no match / differs / false"
// signal, not an error — exempt only that exact case so it isn't shown failed.
// `findstr` is the Windows grep (the shell tool runs via PowerShell there); `find` is
// deliberately absent — it means different things on Windows vs Unix.
const SOFT_FAIL_COMMANDS = new Set(["grep", "egrep", "fgrep", "rg", "findstr", "diff", "cmp", "test", "["]);

function hasToolError(payload: unknown, command: string | undefined) {
  if (!isRecord(payload))
    return false;
  const error = stringValue(payload.error) ?? stringValue(payload.errorText);
  if (error?.trim())
    return true;
  // A shell command that runs is returned as a *successful* tool result (no
  // error field) with the exit code in a footer line at the end of the output
  // ("[exit: N]"). Treat a non-zero code as a failure so the row isn’t
  // shown as completed, except for the soft-fail exemption below.
  const exitCode = nonZeroExitCode(stringValue(payload.text) ?? stringValue(payload.result));
  if (exitCode === null)
    return false;
  return !isSoftExit(exitCode, command);
}

/** The non-zero code from the "[exit: N]" footer line, or null (exit 0 / not a shell result). */
export function nonZeroExitCode(output: string | undefined) {
  if (!output)
    return null;
  const lines = output.trimEnd().split("\n");
  const match = /^\[exit: (-?\d+)\]$/.exec(lines[lines.length - 1] ?? "");
  if (!match)
    return null;
  const code = Number(match[1]);
  return code === 0 ? null : code;
}

export function isSoftExit(exitCode: number, command: string | undefined) {
  // Only exit 1 from a *bare* soft-fail command is exempt. Any shell operator
  // makes the exit code ambiguous (pipeline/list), so those stay failures.
  if (exitCode !== 1 || !command || /[|&;\n`<>]|\$\(/.test(command))
    return false;
  // Basename of the program, tolerant of Windows paths (`\`), a `.exe` suffix,
  // and case (Windows resolves names case-insensitively).
  const token = command.trim().split(/\s+/)[0] ?? "";
  const program = pathBasename(token).toLowerCase().replace(/\.exe$/, "");
  return program ? SOFT_FAIL_COMMANDS.has(program) : false;
}

function stringValue(value: unknown) {
  return typeof value === "string" ? value : undefined;
}
