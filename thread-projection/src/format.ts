import type { AgentMessage } from "./model";

export type RunStatus = "queued" | "running" | "waiting_approval" | "completed" | "failed" | "cancelled";

export function matchesSettledRun(status: RunStatus) {
  return status === "completed" || status === "failed" || status === "cancelled";
}

/**
 * Nearest user message at or before `beforeIndex`, scanning backward. Used to
 * find the exchange that produced a given assistant reply (retry/continue recovery).
 */
export function previousUserMessageBefore(messages: AgentMessage[], beforeIndex: number): AgentMessage | null {
  for (let index = beforeIndex; index >= 0; index -= 1) {
    const message = messages[index];
    if (message?.role === "user")
      return message;
  }
  return null;
}

export interface FriendlyAgentError {
  key: string;
  params?: Record<string, unknown>;
}

/**
 * Pull a human-readable detail out of a raw agent/API error blob: prefer the
 * provider's own `message` field when the error embeds a JSON body, and drop
 * the diagnostic tail ("Request: N messages, N KB.") which is noise for users.
 */
function agentErrorDetail(raw: string): string {
  const embedded = /"message"\s*:\s*"((?:[^"\\]|\\.)*)"/.exec(raw);
  let detail = raw;
  if (embedded) {
    try {
      detail = JSON.parse(`"${embedded[1]}"`) as string;
    }
    catch {
      detail = embedded[1]!;
    }
  }
  // Strip the diagnostic tail ("Request: 2 messages, 16 KB."). The regex avoids
  // `\s*…$`-style backtracking (CodeQL flags it as a potential ReDoS on long
  // input) — match the fixed prefix, then require only trailing whitespace
  // after it before trimming.
  const tail = /Request:\s*\d+\s+messages?,\s*[\d.]+\s*[KM]B\.?/i.exec(detail);
  if (tail) {
    const afterTail = detail.slice((tail.index ?? 0) + tail[0].length);
    if (!/\S/.test(afterTail)) detail = detail.slice(0, tail.index).trimEnd();
  }
  detail = detail.trim();
  const MAX_DETAIL = 300;
  return detail.length > MAX_DETAIL ? `${detail.slice(0, MAX_DETAIL)}…` : detail;
}

/**
 * Classify a raw agent/run error into a user-facing i18n key. Pure (no i18n
 * dependency) so it stays unit-testable; callers translate the returned key.
 * Raw provider errors are developer-oriented dumps ("API request failed (HTTP
 * 402). {\"error\":{…}} Request: 2 messages, 16 KB.") — map the known cases to
 * actionable guidance and clean up the rest.
 */
export function classifyAgentError(raw: string): FriendlyAgentError {
  const message = raw.trim();
  if (!message)
    return { key: "agent:failure.unknown" };
  // Only a genuine gRPC connection failure (prefixed by the Tauri bridge)
  // warrants the "check the agent is running" guidance. Other errors — e.g. the
  // model API rejecting the request (quota / tenant permission) — are run
  // failures, not connectivity problems, and mislabeling them as "connection failure" sends
  // users to debug the wrong thing.
  if (message.includes("Unable to connect to Future Agent"))
    return { key: "agent:failure.connect", params: { message } };
  if (message.includes("[CTX_LIMIT]"))
    return { key: "agent:failure.contextLimit" };

  const lower = message.toLowerCase();
  const status = /\(HTTP (\d{3})\)/.exec(message)?.[1];
  if (status === "402"
    || lower.includes("insufficient credit")
    || lower.includes("balance exhausted")
    || lower.includes("insufficient_quota")) {
    return { key: "agent:failure.insufficientCredit" };
  }
  if (status === "401"
    || status === "403"
    || lower.includes("invalid api key")
    || lower.includes("authentication failed")) {
    return { key: "agent:failure.auth" };
  }
  if (status === "429" || lower.includes("rate limit") || lower.includes("too many requests"))
    return { key: "agent:failure.rateLimited" };
  if (status?.startsWith("5"))
    return { key: "agent:failure.serverError", params: { status } };
  if (/timed out|timeout|etimedout|econnreset|econnrefused|enotfound|network error|fetch failed/i.test(message))
    return { key: "agent:failure.network" };
  return { key: "agent:failure.run", params: { message: agentErrorDetail(message) } };
}
