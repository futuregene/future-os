import { describe, expect, it } from "vitest";
import { classifyAgentError } from "./agentMessageFormatters";

describe("classifyAgentError", () => {
  it("maps an HTTP 402 insufficient-credit rejection to the balance guidance", () => {
    const raw = "API request failed (HTTP 402). {\"error\":{\"code\": \"insufficient credit\", \"type\":\"permission error\",\"message\": \"Balance exhausted. Top up your account or contact your tenant owner.\",\"retryable\":false}} Request: 2 messages, 16 KB.";
    expect(classifyAgentError(raw)).toEqual({ key: "agent:failure.insufficientCredit" });
  });

  it("recognizes quota wording variants without an HTTP status", () => {
    expect(classifyAgentError("Error: insufficient_quota — please check your plan").key)
      .toBe("agent:failure.insufficientCredit");
    expect(classifyAgentError("balance exhausted").key)
      .toBe("agent:failure.insufficientCredit");
  });

  it("maps auth failures (401/403, invalid key) to the credential guidance", () => {
    expect(classifyAgentError("API request failed (HTTP 401). {}").key).toBe("agent:failure.auth");
    expect(classifyAgentError("API request failed (HTTP 403). {}").key).toBe("agent:failure.auth");
    expect(classifyAgentError("invalid api key provided").key).toBe("agent:failure.auth");
  });

  it("maps rate limiting to the wait-and-retry guidance", () => {
    expect(classifyAgentError("API request failed (HTTP 429). {}").key).toBe("agent:failure.rateLimited");
    expect(classifyAgentError("rate limit exceeded").key).toBe("agent:failure.rateLimited");
  });

  it("maps 5xx to the temporary-unavailable guidance, keeping the status", () => {
    expect(classifyAgentError("API request failed (HTTP 503). {}"))
      .toEqual({ key: "agent:failure.serverError", params: { status: "503" } });
  });

  it("maps context-limit errors to the compaction guidance", () => {
    expect(classifyAgentError("[CTX_LIMIT] Request exceeds the model's maximum context length (HTTP 400).").key)
      .toBe("agent:failure.contextLimit");
  });

  it("maps network failures to the connectivity guidance", () => {
    expect(classifyAgentError("request timed out after 60s").key).toBe("agent:failure.network");
    expect(classifyAgentError("fetch failed").key).toBe("agent:failure.network");
  });

  it("keeps the genuine gRPC connection failure as a connect error", () => {
    const raw = "Unable to connect to Future Agent at 127.0.0.1:50051";
    expect(classifyAgentError(raw)).toEqual({ key: "agent:failure.connect", params: { message: raw } });
  });

  it("falls back to the generic run failure with a cleaned detail for unknown errors", () => {
    const result = classifyAgentError("API request failed (HTTP 418). {\"error\":{\"message\":\"teapot broke\"}} Request: 3 messages, 4 KB.");
    expect(result.key).toBe("agent:failure.run");
    // The provider's message field is extracted; the diagnostic tail is dropped.
    expect(result.params?.message).toBe("teapot broke");
  });

  it("strips the request-size diagnostic tail when no JSON message field exists", () => {
    const result = classifyAgentError("API request failed (HTTP 400): code=bad_request, message=\"odd\". Request: 2 messages, 16 KB.");
    expect(result.key).toBe("agent:failure.run");
    expect(result.params?.message).toBe("API request failed (HTTP 400): code=bad_request, message=\"odd\".");
  });

  it("handles empty input", () => {
    expect(classifyAgentError("")).toEqual({ key: "agent:failure.unknown" });
    expect(classifyAgentError("   ")).toEqual({ key: "agent:failure.unknown" });
  });
});
