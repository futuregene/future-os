// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { beforeEach, expect, it, vi } from "vitest";
import { RemoteConversationSettings } from "./RemoteConversationSettings";
import { remoteModelReference } from "./remotePeerClient";
import { useRemoteConversationSettings } from "./useRemoteConversationSettings";

/**
 * The remote conversation's model and thinking level.
 *
 * These act on the *host*, so what is asserted is which command goes where with
 * which value — and that a refusal puts the control back rather than leaving the
 * user believing a setting they do not have.
 */

const requestMock = vi.fn();
vi.mock("./remotePeerClient", async (importOriginal) => {
  const actual = await importOriginal<typeof import("./remotePeerClient")>();
  return {
    ...actual,
    getRemoteConversationSettings: (...args: unknown[]) => requestMock("get_state", ...args),
    listRemoteModels: (...args: unknown[]) => requestMock("list_models", ...args),
    setRemoteConversationModel: (...args: unknown[]) => requestMock("set_model", ...args),
    setRemoteConversationThinkingLevel: (...args: unknown[]) => requestMock("set_thinking_level", ...args),
  };
});

let listeners: Array<(event: { payload: unknown }) => void> = [];
/** Set to hold the next subscription open, so a test can unmount mid-setup. */
let holdListen = false;
let releaseListen: (() => void) | null = null;
vi.mock("@tauri-apps/api/event", () => ({
  listen: async (_name: string, handler: (event: { payload: unknown }) => void) => {
    if (holdListen)
      await new Promise<void>((resolve) => { releaseListen = resolve; });
    listeners.push(handler);
    return () => {
      listeners = listeners.filter(item => item !== handler);
    };
  },
}));

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

const MODELS = [
  { id: "gpt-5", label: "GPT-5", provider: "openai" },
  { id: "auto", label: "Auto", provider: "openrouter" },
];

/** The value the mocked layer answers a given command with. */
function answer(command: string, value: unknown): void {
  requestMock.mockImplementation(async (name: string) => (name === command ? value : undefined));
}

async function mountHook(overrides: {
  desktopId?: string | null;
  enabled?: boolean;
  sessionId?: string | null;
} = {}) {
  const desktopId = overrides.desktopId === undefined ? "desktop_a" : overrides.desktopId;
  const sessionId = overrides.sessionId === undefined ? "sess_1" : overrides.sessionId;
  const enabled = overrides.enabled ?? true;
  let current!: ReturnType<typeof useRemoteConversationSettings>;
  function Probe() {
    current = useRemoteConversationSettings(desktopId, sessionId, enabled);
    return null;
  }
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  await act(async () => root.render(<Probe />));
  await settle();
  return {
    get current() {
      return current;
    },
    container,
    root,
    unmount: async () => {
      await act(async () => root.unmount());
      container.remove();
    },
  };
}

async function settle(): Promise<void> {
  await act(async () => {
    for (let i = 0; i < 8; i += 1) await Promise.resolve();
  });
}

beforeEach(() => {
  listeners = [];
  holdListen = false;
  releaseListen = null;
  requestMock.mockReset();
  requestMock.mockResolvedValue(undefined);
});

it("reads the conversation's settings and the host's catalogue", async () => {
  requestMock.mockImplementation(async (name: string) => {
    if (name === "get_state")
      return { model: "openai/gpt-5", thinkingLevel: "medium" };
    if (name === "list_models")
      return MODELS;
    return undefined;
  });

  const hook = await mountHook();

  expect(hook.current.model).toBe("openai/gpt-5");
  expect(hook.current.thinkingLevel).toBe("medium");
  expect(hook.current.models.map(remoteModelReference)).toEqual(["openai/gpt-5", "openrouter/auto"]);
  await hook.unmount();
});

/** A draft has no conversation on the host, so there is nothing to read or set. */
it("reads nothing for a conversation that does not exist yet", async () => {
  const hook = await mountHook({ sessionId: "" });
  expect(requestMock).not.toHaveBeenCalled();
  await hook.unmount();
});

/**
 * A setting the *host* changes — on its own screen, or by the agent at the start
 * of a run — has to be followed, or the picker shows what the user last chose
 * while the host uses something else.
 */
it("follows the host's own change to the conversation's settings", async () => {
  answer("get_state", { model: "openai/gpt-5", thinkingLevel: "low" });
  const hook = await mountHook();
  expect(hook.current.model).toBe("openai/gpt-5");

  answer("get_state", { model: "openrouter/auto", thinkingLevel: "high" });
  await act(async () => {
    listeners.forEach(listener => listener({
      payload: {
        desktopId: "desktop_a",
        kind: "event",
        payload: { sessionId: "sess_1", type: "model_changed" },
      },
    }));
  });
  await settle();

  expect(hook.current.model).toBe("openrouter/auto");
  await hook.unmount();
});

it("ignores settings events for another conversation or another host", async () => {
  answer("get_state", { model: "openai/gpt-5", thinkingLevel: "low" });
  const hook = await mountHook();
  requestMock.mockClear();

  await act(async () => {
    listeners.forEach(listener => listener({
      payload: { desktopId: "desktop_b", kind: "event", payload: { sessionId: "sess_1", type: "model_changed" } },
    }));
    listeners.forEach(listener => listener({
      payload: { desktopId: "desktop_a", kind: "event", payload: { sessionId: "sess_other", type: "model_changed" } },
    }));
    // A session event that is not a settings change re-reads nothing: the
    // timeline already handles those, and a re-read per token would be a storm.
    listeners.forEach(listener => listener({
      payload: { desktopId: "desktop_a", kind: "event", payload: { sessionId: "sess_1", type: "text_chunk" } },
    }));
  });
  await settle();

  expect(requestMock).not.toHaveBeenCalled();
  await hook.unmount();
});

it("sends a model change with the reference and its provider", async () => {
  answer("get_state", { model: "openai/gpt-5", thinkingLevel: "low" });
  requestMock.mockImplementation(async (name: string) => {
    if (name === "get_state")
      return { model: "openai/gpt-5", thinkingLevel: "low" };
    if (name === "list_models")
      return MODELS;
    return undefined;
  });
  const hook = await mountHook();

  await act(async () => {
    await hook.current.setModel("openrouter/auto");
  });

  expect(requestMock).toHaveBeenCalledWith("set_model", "desktop_a", "sess_1", "openrouter/auto");
  // Shown straight away: waiting for a round trip to move a select looks stuck.
  expect(hook.current.model).toBe("openrouter/auto");
  await hook.unmount();
});

it("sends a thinking-level change", async () => {
  requestMock.mockImplementation(async (name: string) => {
    if (name === "get_state")
      return { model: "openai/gpt-5", thinkingLevel: "low" };
    if (name === "list_models")
      return MODELS;
    return undefined;
  });
  const hook = await mountHook();

  await act(async () => {
    await hook.current.setThinkingLevel("xhigh");
  });

  expect(requestMock).toHaveBeenCalledWith("set_thinking_level", "desktop_a", "sess_1", "xhigh");
  expect(hook.current.thinkingLevel).toBe("xhigh");
  await hook.unmount();
});

/**
 * A refused change goes back to what the host still has, and says why: leaving
 * the optimistic value on screen would show the user a setting they do not have.
 */
it("puts the setting back when the host refuses it", async () => {
  requestMock.mockImplementation(async (name: string) => {
    if (name === "get_state")
      return { model: "openai/gpt-5", thinkingLevel: "low" };
    if (name === "list_models")
      return MODELS;
    if (name === "set_model")
      throw new Error("model_not_available");
    return undefined;
  });
  const hook = await mountHook();

  await act(async () => {
    await hook.current.setModel("openrouter/auto");
  });
  await settle();

  expect(hook.current.error).toContain("model_not_available");
  expect(hook.current.model).toBe("openai/gpt-5");
  await hook.unmount();
});

it("surfaces a failed read", async () => {
  requestMock.mockImplementation(async (name: string) => {
    if (name === "get_state")
      throw new Error("peer_not_connected");
    return undefined;
  });

  const hook = await mountHook();
  expect(hook.current.error).toContain("peer_not_connected");
  await hook.unmount();
});

/** A host with no catalogue leaves the model control unusable rather than empty-but-live. */
it("tolerates a host that answers no catalogue", async () => {
  requestMock.mockImplementation(async (name: string) => {
    if (name === "get_state")
      return { model: "openai/gpt-5", thinkingLevel: null };
    if (name === "list_models")
      throw new Error("agent_unavailable");
    return undefined;
  });

  const hook = await mountHook();
  expect(hook.current.models).toEqual([]);
  // The conversation's own settings still render: only the catalogue failed.
  expect(hook.current.model).toBe("openai/gpt-5");
  await hook.unmount();
});

// ── the control itself ──────────────────────────────────────────────────────

async function mountPicker(settings: Parameters<typeof RemoteConversationSettings>[0]["settings"]) {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  await act(async () => {
    root.render(<RemoteConversationSettings settings={settings} />);
  });
  const select = (label: string) => container.querySelector<HTMLSelectElement>(`select[aria-label="${label}"]`)!;
  return {
    container,
    model: () => select("Model for this conversation (on that computer)"),
    thinking: () => select("Thinking level (on that computer)"),
    choose: async (element: HTMLSelectElement, value: string) => {
      await act(async () => {
        const setter = Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype, "value")!.set!;
        setter.call(element, value);
        element.dispatchEvent(new Event("change", { bubbles: true }));
      });
    },
    unmount: async () => {
      await act(async () => root.unmount());
      container.remove();
    },
  };
}

function state(overrides: Partial<Parameters<typeof useRemoteConversationSettings>[0] extends never ? never : ReturnType<typeof useRemoteConversationSettings>> = {}) {
  return {
    error: null,
    model: "openai/gpt-5",
    models: MODELS,
    saving: false,
    setModel: vi.fn(async () => {}),
    setThinkingLevel: vi.fn(async () => {}),
    thinkingLevel: "low",
    ...overrides,
  };
}

it("offers the host's models, and reports a choice", async () => {
  const settings = state();
  const view = await mountPicker(settings);

  expect(view.model().value).toBe("openai/gpt-5");
  expect([...view.model().options].map(option => option.value)).toEqual(["openai/gpt-5", "openrouter/auto"]);

  await view.choose(view.model(), "openrouter/auto");
  expect(settings.setModel).toHaveBeenCalledWith("openrouter/auto");
  await view.unmount();
});

it("offers the thinking levels, and reports a choice", async () => {
  const settings = state({ thinkingLevel: "off" });
  const view = await mountPicker(settings);

  expect(view.thinking().value).toBe("off");
  await view.choose(view.thinking(), "high");
  expect(settings.setThinkingLevel).toHaveBeenCalledWith("high");
  await view.unmount();
});

/**
 * The host can be using a model its own catalogue does not list — one it
 * resolves itself, or one removed there since. Keeping it as an option is what
 * stops the select from silently showing the first entry as though it were the
 * current model.
 */
it("keeps a current model the catalogue does not list", async () => {
  const view = await mountPicker(state({ model: "internal/house-model" }));

  const values = [...view.model().options].map(option => option.value);
  expect(values[0]).toBe("internal/house-model");
  expect(view.model().value).toBe("internal/house-model");
  await view.unmount();
});

it("disables both controls while a change is in flight", async () => {
  const view = await mountPicker(state({ saving: true }));

  expect(view.model().disabled).toBe(true);
  expect(view.thinking().disabled).toBe(true);
  await view.unmount();
});

/** No catalogue means nothing to choose from, so the control says so by being unusable. */
it("disables the model control when the host listed none", async () => {
  const view = await mountPicker(state({ model: null, models: [] }));

  expect(view.model().disabled).toBe(true);
  // The thinking level is independent of the catalogue and stays usable.
  expect(view.thinking().disabled).toBe(false);
  await view.unmount();
});

/**
 * A reply for a conversation the user has already left must not land.
 *
 * The same hazard the timeline guards against: switching conversations while a
 * read is in flight would otherwise show the previous conversation's model under
 * the new one's title.
 */
it("discards a read whose reply arrives after the user switched", async () => {
  // Every read is held open, and released individually: reassigning one
  // resolver would release the *current* read instead of the stale one, and the
  // test would then be asserting the opposite of what it names.
  const pending: Array<(value: unknown) => void> = [];
  requestMock.mockImplementation((name: string) => {
    if (name !== "get_state")
      return Promise.resolve(MODELS);
    return new Promise((resolve) => {
      pending.push(resolve);
    });
  });

  const desktopId = "desktop_a";
  let session = "sess_1";
  let current!: ReturnType<typeof useRemoteConversationSettings>;
  function Probe() {
    current = useRemoteConversationSettings(desktopId, session, true);
    return null;
  }
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  await act(async () => root.render(<Probe />));

  // The user moves on before the first read answers.
  session = "sess_2";
  await act(async () => root.render(<Probe />));
  await settle();

  // And now the *first* conversation's answer arrives, late, carrying its model.
  expect(pending.length).toBeGreaterThanOrEqual(2);
  await act(async () => {
    pending[0]!({ model: "stale/model", thinkingLevel: "off" });
  });
  await settle();

  expect(current.model).not.toBe("stale/model");
  await act(async () => root.unmount());
  container.remove();
});

it("has no settings to change when there is no conversation", async () => {
  const hook = await mountHook({ sessionId: "" });

  // Both writers are no-ops rather than sending a setting for a conversation
  // that does not exist on the host.
  await act(async () => {
    await hook.current.setModel("openai/gpt-5");
    await hook.current.setThinkingLevel("high");
  });
  expect(requestMock).not.toHaveBeenCalled();
  await hook.unmount();
});

it("does nothing at all while disabled", async () => {
  const hook = await mountHook({ enabled: false });
  expect(requestMock).not.toHaveBeenCalled();
  await hook.unmount();
});

/** A refused thinking-level change goes back too, not just a model one. */
it("puts the thinking level back when the host refuses it", async () => {
  requestMock.mockImplementation(async (name: string) => {
    if (name === "get_state")
      return { model: "openai/gpt-5", thinkingLevel: "low" };
    if (name === "list_models")
      return MODELS;
    if (name === "set_thinking_level")
      throw new Error("unsupported_level");
    return undefined;
  });
  const hook = await mountHook();

  await act(async () => {
    await hook.current.setThinkingLevel("xhigh");
  });
  await settle();

  expect(hook.current.error).toContain("unsupported_level");
  expect(hook.current.thinkingLevel).toBe("low");
  await hook.unmount();
});

/** An event that is not an object cannot be a settings change, so nothing is read. */
it("ignores a malformed event payload", async () => {
  answer("get_state", { model: "openai/gpt-5", thinkingLevel: "low" });
  const hook = await mountHook();
  requestMock.mockClear();

  await act(async () => {
    listeners.forEach(listener => listener({
      payload: { desktopId: "desktop_a", kind: "event", payload: "not an object" },
    }));
    listeners.forEach(listener => listener({
      payload: { desktopId: "desktop_a", kind: "event", payload: null },
    }));
  });
  await settle();

  expect(requestMock).not.toHaveBeenCalled();
  await hook.unmount();
});

/**
 * A subscription that arrives *after* the hook has gone is closed immediately.
 *
 * `listen` is asynchronous, so an unmount during that window leaves a handler
 * alive — and a handler calling back into an unmounted hook is a leak that only
 * shows up as a warning, or as state set on a component that no longer exists.
 */
it("closes a subscription that arrives after it unmounted", async () => {
  holdListen = true;
  const desktopId = "desktop_a";
  function Probe() {
    useRemoteConversationSettings(desktopId, "sess_1", true);
    return null;
  }
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  await act(async () => root.render(<Probe />));

  await act(async () => root.unmount());
  await act(async () => {
    releaseListen?.();
  });
  await settle();

  // Closed, not left in the map: an unmatched listener is the leak.
  expect(listeners).toHaveLength(0);
  container.remove();
});

/**
 * A refused change says why, where the control is.
 *
 * The hook already falls the value back to what the host still has; without the
 * reason beside it the user watches a setting revert with no stated cause, which
 * is indistinguishable from the app ignoring their choice. The state being
 * computed* is not the feature — being on screen is.
 */
it("shows why a refused change was refused", async () => {
  const view = await mountPicker(state({ error: "model_not_available" }));

  expect(view.container.textContent).toContain("model_not_available");
  // Announced, not merely painted: a silent line is easy to miss next to a
  // control the user is still looking at.
  expect(view.container.querySelector("[role='alert']")).not.toBeNull();
  await view.unmount();
});

/** Nothing is announced while nothing has gone wrong. */
it("says nothing when the last change succeeded", async () => {
  const view = await mountPicker(state({ error: null }));

  expect(view.container.querySelector("[role='alert']")).toBeNull();
  await view.unmount();
});
