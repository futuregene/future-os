import { describe, expect, it } from "vitest";
import { classifyAgentError, matchesSettledRun, previousUserMessageBefore } from "./format";
import type { AgentMessage } from "./model";

function message(role: AgentMessage["role"], id: string): AgentMessage {
  return { authorKey: "author.you", content: "", createdAt: "2026-01-01T00:00:00.000Z", id, role };
}

describe("matchesSettledRun", () => {
  it.each([
    ["completed", true],
    ["failed", true],
    ["cancelled", true],
    ["queued", false],
    ["running", false],
    ["waiting_approval", false],
  ] as const)("%s → %s", (status, expected) => {
    expect(matchesSettledRun(status)).toBe(expected);
  });
});

describe("previousUserMessageBefore", () => {
  const messages = [message("user", "u1"), message("assistant", "a1"), message("user", "u2"), message("assistant", "a2")];

  it("scans backward from the given index", () => {
    expect(previousUserMessageBefore(messages, 1)?.id).toBe("u1");
    expect(previousUserMessageBefore(messages, 2)?.id).toBe("u2");
    expect(previousUserMessageBefore(messages, 3)?.id).toBe("u2");
  });

  it("returns null when no user message precedes the index", () => {
    expect(previousUserMessageBefore([message("assistant", "a1")], 0)).toBeNull();
    expect(previousUserMessageBefore(messages, -1)).toBeNull();
    expect(previousUserMessageBefore([], 0)).toBeNull();
    // An out-of-range index reads past the end, skips the holes and keeps
    // scanning backward to the last real user message.
    expect(previousUserMessageBefore(messages, 99)?.id).toBe("u2");
  });
});

describe("classifyAgentError", () => {
  it.each<[string, string]>([
    ["", "agent:failure.unknown"],
    ["   ", "agent:failure.unknown"],
    ["[AGENT_INTERRUPTED] stream closed", "agent:failure.agentInterrupted"],
    ["Unable to connect to Future Agent", "agent:failure.agentInterrupted"],
    ["Unable to send prompt to Future Agent", "agent:failure.agentInterrupted"],
    ["Future Agent event stream", "agent:failure.agentInterrupted"],
    ["Future Agent response timed out", "agent:failure.agentInterrupted"],
    ["Future Agent run ended unexpectedly", "agent:failure.agentInterrupted"],
    ["Future Agent run no longer active", "agent:failure.agentInterrupted"],
    ["Future Agent rejected the prompt", "agent:failure.agentInterrupted"],
    ["prompt acknowledgement omitted", "agent:failure.agentInterrupted"],
    ["[CTX_LIMIT] conversation too long", "agent:failure.contextLimit"],
    ["API request failed (HTTP 402).", "agent:failure.insufficientCredit"],
    ["Insufficient credit for this model", "agent:failure.insufficientCredit"],
    ["balance exhausted", "agent:failure.insufficientCredit"],
    ["insufficient_quota reached", "agent:failure.insufficientCredit"],
    ["API request failed (HTTP 401).", "agent:failure.auth"],
    ["API request failed (HTTP 403).", "agent:failure.auth"],
    ["invalid api key", "agent:failure.auth"],
    ["authentication failed", "agent:failure.auth"],
    ["API request failed (HTTP 429).", "agent:failure.rateLimited"],
    ["rate limit exceeded", "agent:failure.rateLimited"],
    ["too many requests", "agent:failure.rateLimited"],
    ["[RESPONSE_UNCONFIRMED] no terminal event", "agent:failure.responseUnconfirmed"],
    ["[RESPONSE_TIMEOUT] gave up waiting", "agent:failure.responseTimeout"],
    ["[OUTPUT_LIMIT] stop reason length", "agent:failure.outputLimit"],
    ["[MODEL_CONTENT_FILTER] blocked", "agent:failure.contentFilter"],
    ["[MODEL_PAUSED] paused", "agent:failure.modelPaused"],
    ["[PROVIDER_CANCELLED] upstream cancelled", "agent:failure.providerCancelled"],
    ["[SOFTWARE_ERROR] bug", "agent:failure.softwareError"],
    ["session persistence failed", "agent:failure.softwareError"],
    ["failed to persist the turn", "agent:failure.softwareError"],
    ["database is locked", "agent:failure.softwareError"],
    ["database disk image is malformed", "agent:failure.softwareError"],
    ["[UPSTREAM_DISCONNECTED] socket closed", "agent:failure.upstreamDisconnected"],
    ["error decoding response body", "agent:failure.upstreamDisconnected"],
    ["error reading a body", "agent:failure.upstreamDisconnected"],
    ["connection reset by peer", "agent:failure.upstreamDisconnected"],
    ["unexpected eof while reading", "agent:failure.responseUnconfirmed"],
    ["response ended before a clean terminal event", "agent:failure.responseUnconfirmed"],
    ["stream was truncated", "agent:failure.responseUnconfirmed"],
    ["[MODEL_RESPONSE_ERROR] bad json", "agent:failure.modelResponseError"],
    ["invalid provider stream", "agent:failure.modelResponseError"],
    ["invalid provider response", "agent:failure.modelResponseError"],
    ["timed out waiting", "agent:failure.network"],
    ["request timeout", "agent:failure.network"],
    ["ETIMEDOUT", "agent:failure.network"],
    ["ECONNRESET", "agent:failure.network"],
    ["ECONNREFUSED", "agent:failure.network"],
    ["ENOTFOUND", "agent:failure.network"],
    ["network error", "agent:failure.network"],
    ["fetch failed", "agent:failure.network"],
  ])("classifies %j as %s", (raw, key) => {
    expect(classifyAgentError(raw).key).toBe(key);
  });

  it("maps 5xx statuses with the status in the params", () => {
    expect(classifyAgentError("API request failed (HTTP 503).")).toEqual({
      key: "agent:failure.serverError",
      params: { status: "503" },
    });
  });

  it("leaves an unmapped reason code to the generic run failure", () => {
    expect(classifyAgentError("[NOT_A_CODE] something").key).toBe("agent:failure.run");
  });

  it("extracts an embedded provider message and drops the diagnostic tail", () => {
    expect(classifyAgentError('boom. {"error":{"message":"Invalid model id"}} Request: 2 messages, 16 KB.')).toEqual({
      key: "agent:failure.run",
      params: { message: "Invalid model id" },
    });
  });

  it("unescapes the embedded message and falls back when it is not valid JSON", () => {
    expect(classifyAgentError(String.raw`bad {"message":"a\"b"}`).params).toEqual({ message: String.raw`a"b` });
    // `\q` is not a legal JSON escape: the raw capture is kept instead.
    expect(classifyAgentError(String.raw`bad {"message":"a\q"}`).params).toEqual({ message: String.raw`a\q` });
  });

  it("keeps text that follows a diagnostic tail", () => {
    expect(classifyAgentError("first Request: 3 messages, 1.5 MB. second").params).toEqual({
      message: "first Request: 3 messages, 1.5 MB. second",
    });
  });

  it("strips a trailing diagnostic tail from the detail", () => {
    expect(classifyAgentError("oops, the model failed Request: 2 messages, 16 KB.").params).toEqual({
      message: "oops, the model failed",
    });
  });

  it("keeps the tail when it is not trailing whitespace-terminated text", () => {
    expect(classifyAgentError("oops Request: 1 message, 2 KB.x").params).toEqual({
      message: "oops Request: 1 message, 2 KB.x",
    });
  });
  it("truncates a very long detail to 300 characters plus an ellipsis", () => {
    const detail = classifyAgentError("z".repeat(400)).params?.message as string;
    expect(detail).toHaveLength(301);
    expect(detail.endsWith("…")).toBe(true);
    expect(detail.slice(0, 300)).toBe("z".repeat(300));
  });
});
