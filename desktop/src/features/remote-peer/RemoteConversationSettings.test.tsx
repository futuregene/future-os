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

/**
 * A draft has no conversation on the host, but it still has a machine — and a
 * machine has a catalogue.
 *
 * This test used to assert that nothing was read at all, which is exactly the
 * defect: with no catalogue the model picker was empty and therefore disabled, so
 * a conversation started from a workspace heading could not choose a model, and
 * the thinking level fell back to `off` — which is not a default anyone chose.
 * The values seeded here are sent with the first prompt, so what the picker shows
 * is what the host is asked for.
 */
it("seeds a draft from the host's catalogue, and reads no conversation state", async () => {
  requestMock.mockImplementation(async (name: string) => {
    if (name === "list_models") {
      return [
        { id: "gpt-5", label: "GPT-5", provider: "openai" },
        { id: "auto", isDefault: true, label: "Auto", provider: "openrouter" },
      ];
    }
    if (name === "get_state")
      throw new Error("a draft has no state on the host");
    return undefined;
  });

  const hook = await mountHook({ sessionId: "" });

  // The catalogue arrives, so the picker has something to offer.
  expect(hook.current.models.map(remoteModelReference)).toEqual(["openai/gpt-5", "openrouter/auto"]);
  // The host's own designation decides the model, not the order it listed them.
  expect(hook.current.model).toBe("openrouter/auto");
  // And the level a new conversation on that host runs with — the agent's default,
  // not `off`.
  expect(hook.current.thinkingLevel).toBe("medium");
  // No state read: there is no conversation on the host to read it from.
  expect(requestMock.mock.calls.some(call => call[0] === "get_state")).toBe(false);
  await hook.unmount();
});

/** With no default marked, the first model the host offers is what runs. */
it("takes the first model when the host marks no default", async () => {
  requestMock.mockImplementation(async (name: string) =>
    (name === "list_models" ? [{ id: "gpt-5", provider: "openai" }] : undefined));
  const hook = await mountHook({ sessionId: "" });
  expect(hook.current.model).toBe("openai/gpt-5");
  await hook.unmount();
});

/** A model that takes no thinking controls is offered `off`, because that is what it runs. */
it("offers a non-reasoning model no thinking level", async () => {
  requestMock.mockImplementation(async (name: string) =>
    (name === "list_models"
      ? [{ id: "plain", provider: "openai", reasoning: false }]
      : undefined));
  const hook = await mountHook({ sessionId: "" });
  expect(hook.current.thinkingLevel).toBe("off");
  await hook.unmount();
});

/**
 * A draft starts on this app's default, not on the level the host's catalogue
 * declares for the model.
 *
 * That declared level is a capability marker, not a default: the agent reports
 * `high` for every reasoning model and `off` for the rest
 * (`agent/src/rpc/commands/providers.rs`), and this app's own composer ignores it
 * for exactly that reason (`modelThinkingLevel`). Seeding a draft from it made
 * every new remote conversation on a reasoning model run at high effort — the
 * level travels with the first prompt, so the host ran it at high — while the
 * same model started on this machine ran at medium.
 */
it("seeds a draft with this app's default, not the level the host declares", async () => {
  requestMock.mockImplementation(async (name: string) =>
    (name === "list_models"
      ? [{ id: "thinker", provider: "openai", reasoning: true, thinkingLevel: "high" }]
      : undefined));
  const hook = await mountHook({ sessionId: "" });
  expect(hook.current.thinkingLevel).toBe("medium");
  await hook.unmount();
});

/**
 * The other half of that rule: a conversation that already exists on the host
 * shows the level *it* is running at, whatever this app's default is. Only a
 * draft is seeded here; overriding a real session's level would be this machine
 * telling the host what it has.
 */
it("shows the host's own level for a conversation that exists", async () => {
  requestMock.mockImplementation(async (name: string) => {
    if (name === "get_state")
      return { model: "openai/gpt-5", thinkingLevel: "high" };
    if (name === "list_models")
      return [{ id: "gpt-5", provider: "openai", reasoning: true, thinkingLevel: "high" }];
    return undefined;
  });
  const hook = await mountHook();
  expect(hook.current.thinkingLevel).toBe("high");
  await hook.unmount();
});

/** A catalogue that will not load says so, rather than showing an empty picker. */
it("surfaces a catalogue that could not be read for a draft", async () => {
  requestMock.mockImplementation(async (name: string) => {
    if (name === "list_models")
      throw new Error("host is not connected");
    return undefined;
  });
  const hook = await mountHook({ sessionId: "" });
  expect(hook.current.readError).toBe("host is not connected");
  expect(hook.current.model).toBeNull();
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

  expect(hook.current.writeError).toContain("model_not_available");
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
  expect(hook.current.readError).toContain("peer_not_connected");
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
  const MODEL_LABEL = "Model for this conversation (on that computer)";
  const THINKING_LABEL = "Thinking level (on that computer)";
  // This app's own dropdown, not a native `<select>`: a trigger with the current
  // value, whose menu is a listbox of items.
  const trigger = (label: string) => container.querySelector<HTMLButtonElement>(`button[aria-label="${label}"]`)!;
  // Rows inside an open `SelectMenu` panel, which is the absolutely positioned
  // surface every dropdown in this app uses.
  const menuItems = () => [...container.querySelectorAll<HTMLButtonElement>("div.absolute.bottom-9 button")];
  return {
    container,
    menuItems,
    model: () => trigger(MODEL_LABEL),
    modelLabel: () => trigger(MODEL_LABEL).textContent,
    thinking: () => trigger(THINKING_LABEL),
    thinkingLabel: () => trigger(THINKING_LABEL).textContent,
    open: async (label: string) => {
      await act(async () => trigger(label).click());
    },
    /** Open a control and pick the item whose text starts with `text`. */
    choose: async (label: string, text: string) => {
      await act(async () => trigger(label).click());
      const item = menuItems().find(node => (node.textContent ?? "").startsWith(text));
      expect(item, `no menu item starts with ${text}: ${menuItems().map(node => node.textContent).join(" | ")}`).toBeTruthy();
      await act(async () => item!.click());
    },
    unmount: async () => {
      await act(async () => root.unmount());
      container.remove();
    },
  };
}

function state(overrides: Partial<Parameters<typeof useRemoteConversationSettings>[0] extends never ? never : ReturnType<typeof useRemoteConversationSettings>> = {}) {
  return {
    loading: false,
    model: "openai/gpt-5",
    models: MODELS,
    readError: null,
    saving: false,
    setModel: vi.fn(async () => {}),
    setThinkingLevel: vi.fn(async () => {}),
    thinkingLevel: "low",
    writeError: null,
    ...overrides,
  };
}

it("offers the host's models, and reports a choice", async () => {
  const settings = state();
  const view = await mountPicker(settings);

  // The trigger is the current model, at the local composer's shape.
  expect(view.modelLabel()).toContain("GPT-5");
  await view.open("Model for this conversation (on that computer)");
  expect(view.menuItems().map(node => node.textContent)).toEqual(["GPT-5openai", "Autoopenrouter"]);
  await act(async () => view.menuItems()[1]!.click());
  expect(settings.setModel).toHaveBeenCalledWith("openrouter/auto");
  // The menu closes on a choice; the trigger shows the host's own value until it
  // announces the new one.
  expect(view.container.querySelector("div.absolute.bottom-9")).toBeNull();
  await view.unmount();
});

it("offers the thinking levels, and reports a choice", async () => {
  const settings = state({ thinkingLevel: "off" });
  const view = await mountPicker(settings);

  expect(view.thinkingLabel()).toContain("Off");
  await view.choose("Thinking level (on that computer)", "High");
  expect(settings.setThinkingLevel).toHaveBeenCalledWith("high");
  await view.unmount();
});

/**
 * The host can be using a model its own catalogue does not list — one it
 * resolves itself, or one removed there since. Keeping it on screen is what
 * stops the control from silently showing the first entry as though it were the
 * current model.
 */
it("keeps a current model the catalogue does not list", async () => {
  const view = await mountPicker(state({ model: "internal/house-model" }));

  expect(view.modelLabel()).toContain("internal/house-model");
  await view.open("Model for this conversation (on that computer)");
  // It is also selectable, and first — it is the conversation's current value.
  expect(view.menuItems()[0]!.textContent).toContain("internal/house-model");
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

/**
 * On a draft, choosing a model or level moves the value the *prompt* will carry.
 *
 * Nothing is sent to the host — there is no conversation there to set — but the
 * choice has to move in the picker, or the control is dead on a new conversation
 * and the reader watches their choice snap back. `list_models` may have been read
 * (the catalogue); no *writer* may have been called.
 */
it("keeps a draft's choices locally, and sends no setting to the host", async () => {
  requestMock.mockImplementation(async (name: string) =>
    (name === "list_models" ? [{ id: "gpt-5", provider: "openai" }] : undefined));
  const hook = await mountHook({ sessionId: "" });
  requestMock.mockClear();

  await act(async () => {
    await hook.current.setModel("openrouter/auto");
    await hook.current.setThinkingLevel("high");
  });

  expect(hook.current.model).toBe("openrouter/auto");
  expect(hook.current.thinkingLevel).toBe("high");
  expect(requestMock).not.toHaveBeenCalled();
  await hook.unmount();
});

/** And a model that takes no thinking controls turns the level off. */
it("turns thinking off when a draft switches to a model without it", async () => {
  requestMock.mockImplementation(async (name: string) =>
    (name === "list_models"
      ? [{ id: "plain", provider: "openai", reasoning: false }]
      : undefined));
  const hook = await mountHook({ sessionId: "" });

  await act(async () => {
    await hook.current.setModel("openai/plain");
  });
  expect(hook.current.thinkingLevel).toBe("off");
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

  expect(hook.current.writeError).toContain("unsupported_level");
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
  const view = await mountPicker(state({ writeError: "model_not_available" }));

  expect(view.container.textContent).toContain("model_not_available");
  // Announced, not merely painted: a silent line is easy to miss next to a
  // control the user is still looking at.
  expect(view.container.querySelector("[role='alert']")).not.toBeNull();
  await view.unmount();
});

/**
 * A failed *read* is not reported as a failed *change*.
 *
 * The two share the one line under the pickers, and the write wording ("Could
 * not change it") describes an event that did not happen when the host never
 * answered. The reason is the same either way; what changes is which event the
 * user is told about.
 */
it("reports a failed read as a read", async () => {
  const view = await mountPicker(state({ readError: "peer_not_connected" }));

  expect(view.container.textContent).toContain("Could not read it: peer_not_connected");
  expect(view.container.textContent).not.toContain("Could not change it");
  await view.unmount();
});

/** Nothing is announced while nothing has gone wrong. */
it("says nothing when the last change succeeded", async () => {
  const view = await mountPicker(state());

  expect(view.container.querySelector("[role='alert']")).toBeNull();
  await view.unmount();
});

/**
 * The trigger says a *word* while the model is unknown — never its own label.
 *
 * `modelLabel` is the full sentence written for the tooltip and the accessible
 * name. Used as the visible fallback it filled the `max-w-48` trigger with its
 * opening clause ("Model for this conversation (…") for as long as the host
 * took to answer, which on every open is the first thing the reader sees.
 */
it("shows a short word, not its own label, while the model is unknown", async () => {
  const view = await mountPicker(state({ model: null, models: [] }));

  expect(view.modelLabel()?.trim()).toBe("Model");
  // The sentence is still where it belongs: the accessible name and the tooltip.
  expect(view.model().getAttribute("title")).toBe("Model for this conversation (on that computer)");
  await view.unmount();
});

/**
 * A read in flight says so, rather than passing for a conversation with no model.
 *
 * Both are a `null` model, and "Model" on its own reads as the latter — a
 * statement about the *host*, not about this machine waiting for it.
 */
it("says the read is in flight rather than showing the fallback", async () => {
  const view = await mountPicker(state({ loading: true, model: null, models: [] }));

  expect(view.modelLabel()?.trim()).toBe("Loading…");
  await view.unmount();
});

/**
 * A host that lists a model without a label shows its id — the phone's own rule,
 * so the two controls read the same for the same catalogue.
 */
it("falls back to the model's id when the host gives no label", async () => {
  const view = await mountPicker(state({
    model: "openai/gpt-5",
    models: [{ id: "gpt-5", provider: "openai" }],
  }));

  expect(view.modelLabel()?.trim()).toBe("gpt-5");
  await view.unmount();
});

/**
 * With no machine there is nothing to set — and nothing to read.
 *
 * The control is only ever drawn for a conversation that has a machine, so this
 * is the hook's own contract rather than a state the UI reaches: a caller that
 * asks without one gets no request, not a request addressed to nobody.
 */
it("does nothing without a machine", async () => {
  const hook = await mountHook({ desktopId: null });
  expect(requestMock).not.toHaveBeenCalled();

  await act(async () => {
    await hook.current.setModel("openai/gpt-5");
    await hook.current.setThinkingLevel("high");
  });
  expect(requestMock).not.toHaveBeenCalled();
  expect(hook.current.model).toBeNull();
  await hook.unmount();
});
