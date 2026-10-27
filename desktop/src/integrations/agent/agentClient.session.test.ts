// @vitest-environment jsdom
import type { AgentModelOption } from "./agentClient";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  compactThreadContext,
  loadAgentModelOptions,
  modelLabel,
  modelOption,
  modelThinkingLevel,
  modelThinkingLevels,
  normalizeThinkingLevel,
  readLastUsedModel,
  readLastUsedThinkingLevel,
  rememberLastUsedModel,
  rememberLastUsedThinkingLevel,
  resolveInitialModelId,
  resolveInitialThinkingLevel,
  resolveThinkingLevel,
  sendPromptToFutureAgent,
  syncFutureModels,
} from "./agentClient";

const mocks = vi.hoisted(() => ({
  invokeCommand: vi.fn(),
  channelCallbacks: [] as (() => void)[],
}));

vi.mock("@tauri-apps/api/core", () => ({
  Channel: class {
    constructor(callback: () => void) {
      mocks.channelCallbacks.push(callback);
    }
  },
}));
vi.mock("../tauri/invoke", () => ({
  invokeCommand: (...args: unknown[]) => mocks.invokeCommand(...args),
}));

beforeEach(() => {
  mocks.invokeCommand.mockReset();
  mocks.invokeCommand.mockResolvedValue(undefined);
  mocks.channelCallbacks.length = 0;
  window.localStorage.clear();
});

function model(overrides: Partial<AgentModelOption> & Pick<AgentModelOption, "id" | "provider">): AgentModelOption {
  return { label: overrides.id, ...overrides };
}

describe("sendPromptToFutureAgent", () => {
  it("forwards the request verbatim and normalises the response", async () => {
    mocks.invokeCommand.mockResolvedValue({ content: "hello", sessionId: "s1" });

    const result = await sendPromptToFutureAgent({
      message: "hi",
      threadId: "t1",
      attachments: [{ path: "/a.png", kind: "image", name: "a.png" }],
      modelId: "future/gpt-5",
      runId: "r1",
      sessionId: "s1",
      thinkingLevel: "high",
    });

    expect(mocks.invokeCommand).toHaveBeenCalledExactlyOnceWith("agent_prompt", {
      request: {
        attachments: [{ path: "/a.png", kind: "image", name: "a.png" }],
        message: "hi",
        modelContext: "",
        sessionId: "s1",
        threadId: "t1",
        runId: "r1",
        modelId: "future/gpt-5",
        thinkingLevel: "high",
      },
    });
    // An absent `complete` means the stream ended cleanly.
    expect(result).toEqual({ content: "hello", complete: true, sessionId: "s1" });
  });

  it("carries an incomplete stream's termination kind through to the caller", async () => {
    mocks.invokeCommand.mockResolvedValue({
      complete: false,
      content: "partial",
      sessionId: "s1",
      terminationKind: "upstream_disconnected",
    });

    await expect(sendPromptToFutureAgent({ message: "hi", threadId: "t1" })).resolves.toEqual({
      complete: false,
      content: "partial",
      sessionId: "s1",
      terminationKind: "upstream_disconnected",
    });
  });

  it("defaults the optional request fields to null / empty instead of leaving them out", async () => {
    mocks.invokeCommand.mockResolvedValue({ content: "", sessionId: "s1" });

    await sendPromptToFutureAgent({ message: "hi", threadId: "t1" });

    expect(mocks.invokeCommand).toHaveBeenCalledExactlyOnceWith("agent_prompt", {
      request: {
        attachments: [],
        message: "hi",
        modelContext: "",
        sessionId: null,
        threadId: "t1",
        runId: null,
        modelId: null,
        thinkingLevel: null,
      },
    });
  });

  it("registers an acceptance channel only when the caller wants to know, and fires it once", async () => {
    mocks.invokeCommand.mockResolvedValue({ content: "", sessionId: "s1" });
    const onAccepted = vi.fn();

    await sendPromptToFutureAgent({ message: "hi", threadId: "t1", onAccepted });

    expect(mocks.channelCallbacks).toHaveLength(1);
    expect(mocks.invokeCommand.mock.calls[0]![1]).toHaveProperty("onAccepted");
    expect(onAccepted).not.toHaveBeenCalled();

    mocks.channelCallbacks[0]!();
    expect(onAccepted).toHaveBeenCalledTimes(1);
  });

  it("sends no acceptance channel when the caller does not ask for one", async () => {
    mocks.invokeCommand.mockResolvedValue({ content: "", sessionId: "s1" });

    await sendPromptToFutureAgent({ message: "hi", threadId: "t1" });

    expect(mocks.channelCallbacks).toHaveLength(0);
    expect(mocks.invokeCommand.mock.calls[0]![1]).not.toHaveProperty("onAccepted");
  });

  it("propagates a backend refusal (the caller restores the draft)", async () => {
    mocks.invokeCommand.mockRejectedValue(new Error("agent busy"));

    await expect(sendPromptToFutureAgent({ message: "hi", threadId: "t1" })).rejects.toThrow("agent busy");
  });

  it("forwards a CJK message and thread id unchanged", async () => {
    mocks.invokeCommand.mockResolvedValue({ content: "好", sessionId: "s" });

    await sendPromptToFutureAgent({ message: "你好，世界", threadId: "会话-1" });

    expect(mocks.invokeCommand.mock.calls[0]![1].request).toMatchObject({ message: "你好，世界", threadId: "会话-1" });
  });
});

describe("model catalogue helpers", () => {
  it("drops entries without an id and dedupes provider-qualified duplicates", async () => {
    mocks.invokeCommand.mockResolvedValue([
      model({ id: "gpt-5", provider: "future" }),
      model({ id: "gpt-5", provider: "future" }),
      model({ id: "gpt-5", provider: "openai" }),
      model({ id: "   ", provider: "future" }),
      model({ id: "", provider: "future" }),
    ]);

    const models = await loadAgentModelOptions();

    expect(mocks.invokeCommand).toHaveBeenCalledExactlyOnceWith("list_agent_models");
    expect(models.map(m => `${m.provider}/${m.id}`)).toEqual(["future/gpt-5", "openai/gpt-5"]);
  });

  it("keeps an empty catalogue empty", async () => {
    mocks.invokeCommand.mockResolvedValue([]);

    await expect(loadAgentModelOptions()).resolves.toEqual([]);
  });

  it("loads the catalogue in the original order", async () => {
    mocks.invokeCommand.mockResolvedValue([
      model({ id: "b", provider: "p" }),
      model({ id: "a", provider: "p" }),
    ]);

    await expect(loadAgentModelOptions()).resolves.toMatchObject([{ id: "b" }, { id: "a" }]);
  });
});

describe("agent client commands", () => {
  it("compacts a thread and returns the accepted operation", async () => {
    mocks.invokeCommand.mockResolvedValue({ accepted: true, operationId: "op1" });

    await expect(compactThreadContext("t1")).resolves.toEqual({ accepted: true, operationId: "op1" });
    expect(mocks.invokeCommand).toHaveBeenCalledExactlyOnceWith("compact_thread_context", { threadId: "t1" });
  });

  it("propagates a compaction refusal", async () => {
    mocks.invokeCommand.mockRejectedValue(new Error("nothing to compact"));

    await expect(compactThreadContext("t1")).rejects.toThrow("nothing to compact");
  });

  it("syncs the Future models and reports the outcome", async () => {
    const result = { synced: true, modelCount: 4, revision: 12 };
    mocks.invokeCommand.mockResolvedValue(result);

    await expect(syncFutureModels()).resolves.toBe(result);
    expect(mocks.invokeCommand).toHaveBeenCalledExactlyOnceWith("sync_future_models");
  });
});

describe("remembered model and thinking level", () => {
  it("round-trips the last used model", () => {
    rememberLastUsedModel("future/gpt-5");
    expect(readLastUsedModel()).toBe("future/gpt-5");

    // An empty id is not a choice: it must not overwrite the stored one.
    rememberLastUsedModel("");
    expect(readLastUsedModel()).toBe("future/gpt-5");
  });

  it("round-trips the last used thinking level, ignoring an empty one", () => {
    rememberLastUsedThinkingLevel("xhigh");
    expect(readLastUsedThinkingLevel()).toBe("xhigh");

    rememberLastUsedThinkingLevel("");
    expect(readLastUsedThinkingLevel()).toBe("xhigh");
  });

  it("survives an unavailable localStorage on both the write and the read", () => {
    const storage = window.localStorage;
    Object.defineProperty(window, "localStorage", {
      configurable: true,
      get() {
        throw new Error("storage disabled");
      },
    });
    try {
      expect(() => rememberLastUsedModel("future/gpt-5")).not.toThrow();
      expect(readLastUsedModel()).toBeNull();
      expect(() => rememberLastUsedThinkingLevel("high")).not.toThrow();
      expect(readLastUsedThinkingLevel()).toBeNull();
    }
    finally {
      Object.defineProperty(window, "localStorage", { configurable: true, value: storage });
    }
  });

  it("reads a corrupt stored value back verbatim rather than inventing one", () => {
    window.localStorage.setItem("futureos:last-used-model", "not a real model id");
    window.localStorage.setItem("futureos:last-used-thinking-level", "turbo");

    expect(readLastUsedModel()).toBe("not a real model id");
    expect(readLastUsedThinkingLevel()).toBe("turbo");
  });
});

describe("resolveInitialModelId", () => {
  const models = [
    model({ id: "gpt-5", label: "GPT-5", provider: "openai" }),
    model({ id: "deepseek-v4-pro", label: "DS", provider: "future" }),
    model({ id: "other", label: "Other", provider: "future" }),
  ];

  it("prefers a valid remembered model", () => {
    rememberLastUsedModel("openai/gpt-5");

    expect(resolveInitialModelId(models)).toBe("openai/gpt-5");
  });

  it("ignores a remembered model that is no longer in the catalogue", () => {
    rememberLastUsedModel("openai/retired");

    expect(resolveInitialModelId(models)).toBe("future/deepseek-v4-pro");
  });

  it("seeds the remembered model while the catalogue is still empty", () => {
    rememberLastUsedModel("openai/gpt-5");

    expect(resolveInitialModelId([])).toBe("openai/gpt-5");
    window.localStorage.clear();
    expect(resolveInitialModelId([])).toBe("");
  });

  it("prefers Future's deepseek-v4-pro, then any Future model, then the first entry", () => {
    expect(resolveInitialModelId(models)).toBe("future/deepseek-v4-pro");

    const withoutDsv4 = [models[0]!, models[2]!];
    expect(resolveInitialModelId(withoutDsv4)).toBe("future/other");

    const noFuture = [models[0]!];
    expect(resolveInitialModelId(noFuture)).toBe("openai/gpt-5");
  });

  it("falls back to the empty seed for an empty catalogue", () => {
    expect(resolveInitialModelId([])).toBe("");
  });
});

describe("thinking-level helpers", () => {
  // An older agent's catalogue: no `thinkingLevels` list, just the legacy
  // `reasoning` flag and a `defaultThinkingLevel` it clamped from "medium".
  const legacyModels = [
    model({ id: "m", provider: "p", reasoning: true }),
    model({ id: "plain", provider: "p", reasoning: false, thinkingLevels: ["off"], defaultThinkingLevel: "off" }),
  ];

  it("falls back to the legacy six and medium when the catalogue has no list", () => {
    expect(modelThinkingLevels("p/m", legacyModels)).toEqual([
      "off",
      "minimal",
      "low",
      "medium",
      "high",
      "xhigh",
    ]);
    expect(modelThinkingLevel("p/m", legacyModels)).toBe("medium");
    rememberLastUsedThinkingLevel("xhigh");
    expect(resolveInitialThinkingLevel("p/m", legacyModels)).toBe("xhigh");
  });

  it("never sends the literal default to an older agent", () => {
    rememberLastUsedThinkingLevel("default");
    expect(resolveInitialThinkingLevel("p/m", legacyModels)).toBe("medium");
    // Its own list can still narrow the choice in an old-agent shape.
    expect(modelThinkingLevels("p/plain", legacyModels)).toEqual(["off"]);
    expect(modelThinkingLevel("p/plain", legacyModels)).toBe("off");
  });

  it("uses the agent-published list as-is, arbitrary values included", () => {
    const models = [model({
      id: "m",
      provider: "p",
      thinkingLevels: ["default", "off", "max"],
      defaultThinkingLevel: "default",
    })];
    expect(modelThinkingLevels("p/m", models)).toEqual(["default", "off", "max"]);
    expect(modelThinkingLevel("p/m", models)).toBe("default");

    rememberLastUsedThinkingLevel("max");
    expect(resolveInitialThinkingLevel("p/m", models)).toBe("max");
    // A remembered level the model does not offer falls back to its own start.
    rememberLastUsedThinkingLevel("high");
    expect(resolveInitialThinkingLevel("p/m", models)).toBe("default");
  });

  it("starts a narrowed model on the level its agent computed", () => {
    const models = [model({
      id: "narrow",
      provider: "p",
      thinkingLevels: ["default", "low", "high"],
      defaultThinkingLevel: "low",
    })];
    expect(modelThinkingLevel("p/narrow", models)).toBe("low");
    expect(resolveInitialThinkingLevel("p/narrow", models)).toBe("low");
    expect(resolveThinkingLevel("xhigh", "p/narrow", models)).toBe("low");
    expect(resolveThinkingLevel("high", "p/narrow", models)).toBe("high");
  });

  it("keeps a single-level model on its only level", () => {
    const models = [model({
      id: "plain",
      provider: "p",
      thinkingLevels: ["default"],
      defaultThinkingLevel: "default",
    })];
    expect(modelThinkingLevels("p/plain", models)).toEqual(["default"]);
    expect(modelThinkingLevel("p/plain", models)).toBe("default");
    expect(resolveThinkingLevel("off", "p/plain", models)).toBe("default");
  });

  it("normalises outside values by emptiness alone; any token is a level", () => {
    expect(normalizeThinkingLevel("max")).toBe("max");
    expect(normalizeThinkingLevel("nonsense")).toBe("nonsense");
    expect(normalizeThinkingLevel("  ")).toBe("default");
    expect(normalizeThinkingLevel(null)).toBe("default");
    expect(normalizeThinkingLevel(undefined)).toBe("default");
  });
});

describe("model lookup", () => {
  const models = [
    model({ id: "gpt-5", label: "GPT-5", provider: "openai" }),
    model({ id: "gpt-5", label: "GPT-5 Future", provider: "future" }),
    model({ id: "legacy/model", label: "Legacy", provider: "" }),
  ];

  it("prefers an exact provider-qualified match", () => {
    expect(modelOption("future/gpt-5", models)?.label).toBe("GPT-5 Future");
    expect(modelOption("openai/gpt-5", models)?.label).toBe("GPT-5");
  });

  it("resolves a unique bare id, and refuses an ambiguous one", () => {
    expect(modelOption("legacy/model", models)?.label).toBe("Legacy");
    // "gpt-5" exists under two providers: guessing could pick the wrong endpoint.
    expect(modelOption("gpt-5", models)).toBeUndefined();
    expect(modelOption("missing", models)).toBeUndefined();
  });

  it("labels a model, falling back to the id and then to undefined", () => {
    expect(modelLabel("future/gpt-5", models)).toBe("GPT-5 Future");
    expect(modelLabel("unknown-id", models)).toBe("unknown-id");
    expect(modelLabel("", models)).toBeUndefined();
  });
});
