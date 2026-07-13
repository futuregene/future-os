import type { AgentMessage, MessageSegment } from "./agentThreadTypes";

/** Raw entry from agent get_session_entries RPC. */
export interface SessionEntry {
  id: string;
  role: "user" | "assistant" | "tool";
  content: string;
  name?: string;
  tool_args?: string;
  thinking?: string;
  tool_calls?: Array<{ function: { name: string; arguments: unknown } }>;
  /** RFC3339 entry time; preserved across re-saves so history keeps real times. */
  timestamp?: string;
  /** Output tokens for the reply — only the final assistant entry of a run. */
  output_tokens?: number;
  /** Run wall-clock duration in ms — paired with `output_tokens`. */
  duration_ms?: number;
}

const TOOL_NAMES = new Set(["read", "bash", "edit", "write"]);

function asToolKind(name: string): "read" | "bash" | "edit" | "write" {
  return TOOL_NAMES.has(name) ? (name as "read" | "bash" | "edit" | "write") : "bash";
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
}

let _seq = 0;
function segId(): string {
  return `ep_${Date.now()}_${++_seq}`;
}

/**
 * Convert raw agent session entries into AgentMessage[] for the GUI pipeline.
 * Each user→assistant turn yields 1 user + 1 assistant message with segments
 * for thinking, tool activity, and text.
 */
export function entriesToMessages(entries: SessionEntry[]): AgentMessage[] {
  const messages: AgentMessage[] = [];
  const now = new Date().toISOString();
  let acc: TurnAcc | null = null;

  function flush() {
    if (!acc?.userMessage)
      return;
    messages.push(acc.userMessage);
    const textSegments = acc.segments.filter(s => s.kind === "text") as { kind: "text"; id: string; text: string }[];
    messages.push({
      id: segId(),
      role: "assistant",
      authorKey: "author.researchCopilot",
      content: acc.finalText || textSegments.map(s => s.text).join("\n"),
      segments: acc.segments.length > 0 ? acc.segments : undefined,
      status: "complete",
      createdAt: acc.assistantCreatedAt ?? now,
      outputTokens: acc.outputTokens,
      durationMs: acc.durationMs,
    });
    acc = null;
  }

  for (const entry of entries) {
    if (entry.role === "user") {
      if (acc)
        flush();
      acc = { segments: [], finalText: "" };
      acc.userMessage = {
        id: segId(),
        role: "user",
        authorKey: "author.you",
        content: entry.content,
        status: "complete",
        createdAt: entry.timestamp ?? now,
      };
    }
    else if (entry.role === "assistant") {
      if (!acc)
        acc = { segments: [], finalText: "" };
      // Last assistant entry of the turn carries the reply's time + usage.
      if (entry.timestamp)
        acc.assistantCreatedAt = entry.timestamp;
      if (typeof entry.output_tokens === "number")
        acc.outputTokens = entry.output_tokens;
      if (typeof entry.duration_ms === "number")
        acc.durationMs = entry.duration_ms;
      if (entry.thinking) {
        acc.segments.push({ id: segId(), kind: "thinking", text: entry.thinking });
      }
      if (entry.tool_calls) {
        for (const tc of entry.tool_calls) {
          acc.segments.push({
            id: segId(),
            kind: "activity",
            item: {
              id: segId(),
              kind: asToolKind(tc.function.name),
              status: "completed",
              detail: typeof tc.function.arguments === "string" ? tc.function.arguments : JSON.stringify(tc.function.arguments),
            },
          });
        }
      }
      if (entry.content?.trim()) {
        acc.segments.push({ id: segId(), kind: "text", text: entry.content });
        acc.finalText = entry.content;
      }
    }
    else if (entry.role === "tool") {
      if (!acc)
        acc = { segments: [], finalText: "" };
      acc.segments.push({
        id: segId(),
        kind: "activity",
        item: {
          id: segId(),
          kind: asToolKind(entry.name || "bash"),
          status: "completed",
          detail: entry.tool_args || "",
        },
      });
    }
  }
  if (acc)
    flush();
  return messages;
}
