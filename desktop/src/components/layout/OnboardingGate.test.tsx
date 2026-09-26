// @vitest-environment jsdom
import type { ReactElement } from "react";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import i18n from "../../i18n";
import { OnboardingGate } from "./OnboardingGate";

// React only flushes state updates inside `act` when it is told the environment
// supports it; without this the async init flow would never commit.
(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

const mocks = vi.hoisted(() => ({
  begin: vi.fn(async () => {}),
  bootstrapBuiltinSkills: vi.fn(async () => {}),
  cancel: vi.fn(),
  env: { environment: "production" } as { environment: string } | null,
  envPending: null as Promise<{ environment: string } | null> | null,
  invoke: vi.fn(async (_command: string, _args?: unknown) => {}),
  isRelease: false,
  loadModels: vi.fn(async () => [] as unknown[]),
  loginFlow: { message: null as string | null, phase: "idle" as string },
  rememberLastUsedModel: vi.fn(),
  syncFutureModels: vi.fn(async () => {}),
}));

vi.mock("../../features/settings/useFutureLoginFlow", () => ({
  useFutureLoginFlow: () => ({
    begin: mocks.begin,
    cancel: mocks.cancel,
    message: mocks.loginFlow.message,
    phase: mocks.loginFlow.phase,
  }),
}));
vi.mock("../../integrations/tauri/useBuildInfo", () => ({
  useBuildInfo: () => ({ data: { isRelease: mocks.isRelease, version: "9.9.9" } }),
}));
vi.mock("../../integrations/tauri/invoke", () => ({
  invokeCommand: (command: string, args?: unknown) => mocks.invoke(command, args),
}));
vi.mock("../../integrations/agent/providers", () => ({
  getFutureEnvironment: () => mocks.envPending ?? Promise.resolve(mocks.env),
}));
vi.mock("../../integrations/skills/skillsClient", () => ({
  bootstrapBuiltinSkills: () => mocks.bootstrapBuiltinSkills(),
}));
vi.mock("../../integrations/agent/agentClient", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../../integrations/agent/agentClient")>();
  return {
    ...actual,
    loadAgentModelOptions: () => mocks.loadModels(),
    rememberLastUsedModel: (modelId: string) => mocks.rememberLastUsedModel(modelId),
    syncFutureModels: () => mocks.syncFutureModels(),
  };
});

function model(id: string, extra: Record<string, unknown> = {}) {
  return ({ id, label: `Model ${id}`, provider: "future", ...extra }) as never;
}

function mount(node: ReactElement) {
  const container = document.createElement("div");
  document.body.appendChild(container);
  const root = createRoot(container);
  act(() => root.render(node));
  return {
    container,
    rerender: (next: ReactElement) => act(() => root.render(next)),
    unmount: () => {
      act(() => root.unmount());
      container.remove();
    },
  };
}

function baseProps(overrides: Record<string, unknown> = {}) {
  return {
    hasAnyProvider: false,
    initPending: false,
    modelsReady: false,
    onCancelLogin: vi.fn(),
    onEnableBYOK: vi.fn(),
    onInitComplete: vi.fn(),
    ...overrides,
  };
}

function buttonByText(container: HTMLElement, text: string) {
  return [...container.querySelectorAll<HTMLButtonElement>("button")].find(button => button.textContent?.includes(text));
}

/** Let the init flow run to completion under fake timers. */
async function runInitToCompletion(steps = 20, ms = 2000) {
  for (let index = 0; index < steps; index++) {
    await act(async () => {
      await vi.advanceTimersByTimeAsync(ms);
    });
  }
}

beforeEach(() => {
  mocks.begin.mockClear();
  mocks.bootstrapBuiltinSkills.mockReset().mockResolvedValue(undefined);
  mocks.cancel.mockClear();
  mocks.env = { environment: "production" };
  mocks.envPending = null;
  mocks.invoke.mockReset().mockResolvedValue(undefined);
  mocks.isRelease = false;
  mocks.loadModels.mockReset().mockResolvedValue([]);
  mocks.loginFlow = { message: null, phase: "idle" };
  mocks.rememberLastUsedModel.mockClear();
  mocks.syncFutureModels.mockReset().mockResolvedValue(undefined);
  vi.useFakeTimers();
});
afterEach(() => {
  vi.useRealTimers();
});

describe("onboarding gate login states", () => {
  it("offers sign-in and bring-your-own-key on the idle gate", () => {
    const p = baseProps();
    const view = mount(<OnboardingGate {...p} />);
    expect(view.container.textContent).toContain("Welcome to FutureOS");
    const login = buttonByText(view.container, "Sign in / Sign up")!;
    expect(login.disabled).toBe(false);
    act(() => login.click());
    expect(mocks.begin).toHaveBeenCalledTimes(1);

    act(() => buttonByText(view.container, "Use your own API key")!.click());
    expect(mocks.cancel).toHaveBeenCalledTimes(1);
    expect(p.onEnableBYOK).toHaveBeenCalledTimes(1);
    view.unmount();
  });

  it("locks the gate and offers Cancel while a login is in flight", () => {
    // `authorized` is deliberately excluded: that phase hands straight over to
    // the init flow (covered under initialization below).
    for (const phase of ["starting", "waiting"]) {
      mocks.loginFlow = { message: null, phase };
      const p = baseProps({ hasAnyProvider: true });
      const view = mount(<OnboardingGate {...p} />);
      const busy = buttonByText(view.container, "Signing in")!;
      expect(busy.disabled).toBe(true);
      expect(buttonByText(view.container, "Sign in / Sign up")).toBeUndefined();
      // The hint and divider keep their space (invisible) so the layout cannot
      // jitter when BYOK swaps for Cancel.
      expect(view.container.querySelector("p.invisible")).not.toBeNull();
      expect(view.container.querySelector("div.invisible")).not.toBeNull();

      act(() => buttonByText(view.container, "Cancel")!.click());
      expect(mocks.cancel).toHaveBeenCalled();
      // A provider already exists, so cancelling closes the gate outright.
      expect(p.onEnableBYOK).toHaveBeenCalledTimes(1);
      expect(p.onCancelLogin).not.toHaveBeenCalled();
      view.unmount();
    }
  });

  it("resets to the initial state when cancelling with no provider", () => {
    mocks.loginFlow = { message: null, phase: "waiting" };
    const p = baseProps({ hasAnyProvider: false });
    const view = mount(<OnboardingGate {...p} />);
    act(() => buttonByText(view.container, "Cancel")!.click());
    expect(p.onCancelLogin).toHaveBeenCalledTimes(1);
    expect(p.onEnableBYOK).not.toHaveBeenCalled();
    view.unmount();
  });

  it("shows the failure message and a retry label for each terminal failure", () => {
    for (const phase of ["denied", "expired", "error"]) {
      mocks.loginFlow = { message: "Device code expired", phase };
      const view = mount(<OnboardingGate {...baseProps()} />);
      expect(view.container.textContent).toContain("Device code expired");
      expect(buttonByText(view.container, "Retry")).toBeTruthy();
      view.unmount();
    }
  });

  it("falls back to the generic failure copy when no message is available", () => {
    mocks.loginFlow = { message: null, phase: "error" };
    const view = mount(<OnboardingGate {...baseProps()} />);
    expect(view.container.textContent).toContain("Authorization failed.");
    view.unmount();
  });

  it("starts the login flow automatically when opened from Settings, once", () => {
    const view = mount(<OnboardingGate {...baseProps({ autoLogin: true })} />);
    expect(mocks.begin).toHaveBeenCalledTimes(1);
    view.rerender(<OnboardingGate {...baseProps({ autoLogin: true })} />);
    expect(mocks.begin).toHaveBeenCalledTimes(1);
    view.unmount();
  });

  it("offers the shipped languages in the corner switcher", () => {
    const view = mount(<OnboardingGate {...baseProps()} />);
    const select = view.container.querySelector<HTMLSelectElement>("select")!;
    const options = [...select.querySelectorAll<HTMLOptionElement>("option")].map(option => option.value);
    expect(options).toContain("en");
    expect(options).toContain("zh");

    try {
      act(() => {
        Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype, "value")!.set!.call(select, "zh");
        select.dispatchEvent(new Event("change", { bubbles: true }));
      });
      expect(i18n.language).toBe("zh");
    }
    finally {
      void i18n.changeLanguage("en");
      view.unmount();
    }
  });
});

describe("onboarding gate initialization", () => {
  it("runs the three init steps and finishes with a single recommended model", async () => {
    mocks.loadModels.mockResolvedValue([model("solo", { recommended: true })]);
    const p = baseProps({ initPending: true, modelsReady: true });
    const view = mount(<OnboardingGate {...p} />);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });
    // Step 0 resolved immediately; the progress copy names the current step.
    expect(mocks.loadModels).toHaveBeenCalled();
    await runInitToCompletion();

    expect(mocks.syncFutureModels).toHaveBeenCalledTimes(1);
    expect(mocks.bootstrapBuiltinSkills).toHaveBeenCalledTimes(1);
    expect(mocks.invoke).toHaveBeenCalledWith("set_default_model", { modelId: "future/solo" });
    expect(mocks.rememberLastUsedModel).toHaveBeenCalledWith("future/solo");
    expect(p.onInitComplete).toHaveBeenCalledTimes(1);
    view.unmount();
  });

  it("closes the gate without a model write when nothing is recommended", async () => {
    mocks.loadModels.mockResolvedValue([model("plain")]);
    const p = baseProps({ initPending: true, modelsReady: true });
    const view = mount(<OnboardingGate {...p} />);
    await runInitToCompletion();

    expect(mocks.invoke).not.toHaveBeenCalled();
    expect(p.onInitComplete).toHaveBeenCalledTimes(1);
    view.unmount();
  });

  it("still finishes when the default-model write fails", async () => {
    mocks.loadModels.mockResolvedValue([model("solo", { recommended: true })]);
    mocks.invoke.mockRejectedValue(new Error("agent busy"));
    const p = baseProps({ initPending: true, modelsReady: true });
    const view = mount(<OnboardingGate {...p} />);
    await runInitToCompletion();

    expect(p.onInitComplete).toHaveBeenCalledTimes(1);
    view.unmount();
  });

  it("survives a failing catalog sync and a failing skills bootstrap", async () => {
    mocks.loadModels.mockResolvedValue([model("solo", { recommended: true })]);
    mocks.syncFutureModels.mockRejectedValue(new Error("no catalog"));
    mocks.bootstrapBuiltinSkills.mockRejectedValue(new Error("no skills"));
    const p = baseProps({ initPending: true, modelsReady: true });
    const view = mount(<OnboardingGate {...p} />);
    await runInitToCompletion();

    expect(mocks.bootstrapBuiltinSkills).toHaveBeenCalledTimes(1);
    expect(p.onInitComplete).toHaveBeenCalledTimes(1);
    view.unmount();
  });

  it("retries the reachability probe until the agent answers", async () => {
    mocks.loadModels
      .mockRejectedValueOnce(new Error("unable to connect to Future Agent"))
      .mockResolvedValue([model("solo", { recommended: true })]);
    const p = baseProps({ initPending: true, modelsReady: true });
    const view = mount(<OnboardingGate {...p} />);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });
    // The first probe fails, so the init waits out the retry delay.
    expect(mocks.loadModels).toHaveBeenCalledTimes(1);
    await runInitToCompletion(8, 1300);
    expect(mocks.loadModels.mock.calls.length).toBeGreaterThan(1);
    expect(p.onInitComplete).toHaveBeenCalled();
    view.unmount();
  });

  it("gives up on an empty catalog after the confirmation deadline and closes", async () => {
    mocks.loadModels.mockResolvedValue([]);
    const p = baseProps({ initPending: true });
    const view = mount(<OnboardingGate {...p} />);
    await runInitToCompletion(30, 1000);

    // Never confirmed, so the gate closes with no model choice at all.
    expect(p.onInitComplete).toHaveBeenCalledTimes(1);
    expect(mocks.invoke).not.toHaveBeenCalled();
    view.unmount();
  });

  it("shows the model picker for two or more recommended models and applies the pick", async () => {
    mocks.loadModels.mockResolvedValue([
      model("first", { recommended: true }),
      model("second", { recommended: true }),
      model("third"),
    ]);
    const p = baseProps({ initPending: true, modelsReady: true });
    const view = mount(<OnboardingGate {...p} />);
    await runInitToCompletion();

    expect(view.container.textContent).toContain("Choose your AI model");
    const cards = [...view.container.querySelectorAll<HTMLButtonElement>("button")]
      .filter(button => button.textContent === "Model first" || button.textContent === "Model second");
    // Only the recommended ones are offered (the third is not a candidate).
    expect(cards.length).toBe(2);

    act(() => cards[1]!.click());
    await act(async () => {
      await buttonByText(view.container, "Get started")!.click();
      await Promise.resolve();
    });
    expect(mocks.invoke).toHaveBeenCalledWith("set_default_model", { modelId: "future/second" });
    expect(p.onInitComplete).toHaveBeenCalledTimes(1);
    view.unmount();
  });

  it("shows each model's localized description in the picker", async () => {
    // A description is optional and locale-dependent: `localizedModelDescription`
    // prefers `description` in Chinese and `descriptionEn` in English, and falls
    // back to the other when the preferred one is blank. Models without any
    // description render the card without the caption.
    mocks.loadModels.mockResolvedValue([
      model("first", { description: "中文简介", descriptionEn: "English blurb", recommended: true }),
      model("second", { description: "   ", descriptionEn: "Fallback blurb", recommended: true }),
      model("third", { recommended: true }),
    ]);
    const p = baseProps({ initPending: true, modelsReady: true });
    const view = mount(<OnboardingGate {...p} />);
    await runInitToCompletion();

    expect(view.container.textContent).toContain("Choose your AI model");
    // A model with a description shows it…
    expect(view.container.textContent).toContain("English blurb");
    // …a blank preferred description falls back to the other locale's text…
    expect(view.container.textContent).toContain("Fallback blurb");
    // …and a model with no description at all renders without a caption.
    const third = [...view.container.querySelectorAll<HTMLButtonElement>("button")]
      .find(button => button.textContent === "Model third")!;
    expect(third.querySelector(".line-clamp-2")).toBeNull();
    view.unmount();
  });

  it("pre-selects the first recommended model, and confirms it when the user does not choose", async () => {
    // Guards the invariant that keeps two arms of `handleStart`'s
    // `find(…) ?? recommendedModels[0] ?? null` dead: the finalize effect seeds
    // `selectedModelId` from `recommendedModels[0]`, so `find` always succeeds.
    //
    // The invariant is user-visible twice over, which is why it is worth locking:
    // a bogus/absent seed leaves the picker with **no card highlighted**, and the
    // confirmed model silently falls through to `recommendedModels[0]`. Mutation-
    // checked (§2b fault 22): seeding a key that is not in the list leaves 24/24
    // tests green *before* this test existed, while making the uncovered arm
    // reachable — i.e. the invariant was real but unguarded.
    mocks.loadModels.mockResolvedValue([
      model("first", { recommended: true }),
      model("second", { recommended: true }),
    ]);
    const p = baseProps({ initPending: true, modelsReady: true });
    const view = mount(<OnboardingGate {...p} />);
    await runInitToCompletion();

    // Exactly one card is highlighted, and it is the first recommended model.
    const highlighted = [...view.container.querySelectorAll<HTMLButtonElement>("button")]
      .filter(button => button.className.includes("bg-accent-soft") && button.textContent?.startsWith("Model "));
    expect(highlighted.map(button => button.textContent)).toEqual(["Model first"]);

    // Confirming without touching the picker therefore applies that same model.
    await act(async () => {
      await buttonByText(view.container, "Get started")!.click();
      await Promise.resolve();
    });
    expect(mocks.invoke).toHaveBeenCalledWith("set_default_model", { modelId: "future/first" });
    expect(p.onInitComplete).toHaveBeenCalledTimes(1);
    view.unmount();
  });

  it("applies only once even if Get started is activated again in flight", async () => {
    mocks.loadModels.mockResolvedValue([
      model("first", { recommended: true }),
      model("second", { recommended: true }),
    ]);
    let release!: () => void;
    mocks.invoke.mockReturnValue(new Promise<void>((resolve) => {
      release = resolve;
    }));
    const p = baseProps({ initPending: true, modelsReady: true });
    const view = mount(<OnboardingGate {...p} />);
    await runInitToCompletion();

    const start = buttonByText(view.container, "Get started")!;
    await act(async () => {
      start.click();
      await Promise.resolve();
    });
    expect(start.disabled).toBe(true);

    // A second activation must be rejected by the in-flight guard itself, not
    // only by the disabled attribute: React suppresses click delivery to
    // disabled form controls, so the attribute is removed here to reach the
    // handler the way a lost/overridden disabled state would.
    await act(async () => {
      start.removeAttribute("disabled");
      start.dispatchEvent(new MouseEvent("click", { bubbles: true }));
      await Promise.resolve();
    });
    expect(mocks.invoke).toHaveBeenCalledTimes(1);

    await act(async () => {
      release();
      await Promise.resolve();
    });
    expect(p.onInitComplete).toHaveBeenCalledTimes(1);
    view.unmount();
  });

  it("waits for the live catalog before finalizing the gate", async () => {
    mocks.loadModels.mockResolvedValue([
      model("first", { recommended: true }),
      model("second", { recommended: true }),
    ]);
    const p = baseProps({ initPending: true, modelsReady: false });
    const view = mount(<OnboardingGate {...p} />);
    await runInitToCompletion();

    // Models were confirmed during init, but the app's own catalog hook has not
    // caught up yet — the gate must stay up rather than flashing the composer's
    // "no models configured" banner.
    expect(view.container.textContent).toContain("Ready");
    expect(p.onInitComplete).not.toHaveBeenCalled();

    // Once the live catalog agrees, the picker for the >= 2 recommended models
    // appears and the gate still waits for the user's choice.
    view.rerender(<OnboardingGate {...p} modelsReady />);
    await act(async () => {
      await Promise.resolve();
    });
    expect(view.container.textContent).toContain("Choose your AI model");
    expect(p.onInitComplete).not.toHaveBeenCalled();

    // A later "not caught up" blip re-runs the finalize effect, which must not
    // pick a model a second time once the choice has already been presented.
    view.rerender(<OnboardingGate {...p} modelsReady={false} />);
    await act(async () => {
      await Promise.resolve();
    });
    view.rerender(<OnboardingGate {...p} modelsReady />);
    await act(async () => {
      await Promise.resolve();
    });
    expect(view.container.textContent).toContain("Choose your AI model");
    expect(p.onInitComplete).not.toHaveBeenCalled();
    view.unmount();
  });

  it("starts the init flow when the login flow reaches the authorized phase", async () => {
    mocks.loadModels.mockResolvedValue([model("solo", { recommended: true })]);
    mocks.loginFlow = { message: null, phase: "authorized" };
    const p = baseProps({ initPending: false, modelsReady: true });
    const view = mount(<OnboardingGate {...p} />);
    await runInitToCompletion();

    expect(mocks.bootstrapBuiltinSkills).toHaveBeenCalledTimes(1);
    expect(p.onInitComplete).toHaveBeenCalledTimes(1);
    view.unmount();
  });

  it("waits out the minimum init duration instead of flashing the progress bar", async () => {
    mocks.loadModels.mockResolvedValue([model("solo", { recommended: true })]);
    const p = baseProps({ initPending: true, modelsReady: true });
    const view = mount(<OnboardingGate {...p} />);

    // Let the init work resolve: the three steps are done, but the gate is still
    // inside the 500 ms floor, so it has not finalized.
    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });
    expect(p.onInitComplete).not.toHaveBeenCalled();
    expect(view.container.textContent).toContain("/ 3");

    await act(async () => {
      await vi.advanceTimersByTimeAsync(600);
    });
    expect(p.onInitComplete).toHaveBeenCalledTimes(1);
    view.unmount();
  });
});

describe("onboarding gate environment switcher", () => {
  const environmentSelect = (container: HTMLElement) =>
    [...container.querySelectorAll<HTMLSelectElement>("select")]
      .find(select => [...select.options].some(option => option.value === "production" && option.textContent === "Production"));

  it("shows a loading placeholder and stays disabled until the environment probe lands", async () => {
    // The dev switcher has three sub-states, and only two were covered: the
    // `custom` placeholder once the probe answers with something other than
    // test/production, and `disabled` while a *switch* is in flight. The
    // **loading** sub-state was unasserted — while `getFutureEnvironment` is
    // still pending the placeholder reads "..." (not "custom", which would
    // wrongly imply the environment is already known) and the control is
    // disabled so the user cannot pick against a stale option list.
    let releaseEnv!: (value: { environment: string }) => void;
    mocks.envPending = new Promise((resolve) => {
      releaseEnv = resolve;
    });

    const view = mount(<OnboardingGate {...baseProps()} />);
    await act(async () => {
      await Promise.resolve();
    });

    // Loading: placeholder is "..." and the select cannot be used yet.
    let select = environmentSelect(view.container)!;
    const loadingPlaceholder = [...select.options].find(option => option.value === "")!;
    expect(loadingPlaceholder.textContent).toBe("...");
    expect(select.disabled).toBe(true);

    // The probe resolves to an environment that is neither test nor production,
    // so the placeholder becomes the truthful "custom" label and the control
    // becomes usable.
    await act(async () => {
      releaseEnv({ environment: "custom" });
      await Promise.resolve();
      await Promise.resolve();
    });

    select = environmentSelect(view.container)!;
    const settledPlaceholder = [...select.options].find(option => option.value === "")!;
    expect(settledPlaceholder.textContent).toBe("custom");
    expect(select.disabled).toBe(false);
    view.unmount();
  });

  it("is hidden on a release build", async () => {
    mocks.isRelease = true;
    const view = mount(<OnboardingGate {...baseProps()} />);
    await act(async () => {
      await Promise.resolve();
    });
    expect(view.container.textContent).not.toContain("Environment");
    view.unmount();
  });

  it("switches the environment and stays disabled until the change lands", async () => {
    let release!: () => void;
    mocks.invoke.mockReturnValue(new Promise<void>((resolve) => {
      release = resolve;
    }));
    const view = mount(<OnboardingGate {...baseProps()} />);
    await act(async () => {
      await Promise.resolve();
    });
    const select = environmentSelect(view.container)!;
    expect(select.value).toBe("production");

    act(() => {
      select.value = "test";
      select.dispatchEvent(new Event("change", { bubbles: true }));
    });
    expect(mocks.invoke).toHaveBeenCalledWith("set_future_environment", { environment: "test" });
    expect(environmentSelect(view.container)!.disabled).toBe(true);
    expect(view.container.querySelector(".animate-spin")).not.toBeNull();

    await act(async () => {
      release();
      await Promise.resolve();
    });
    view.unmount();
  });

  it("re-enables the switcher and reports the failure when the switch fails", async () => {
    mocks.invoke.mockRejectedValue(new Error("agent offline"));
    const view = mount(<OnboardingGate {...baseProps()} />);
    await act(async () => {
      await Promise.resolve();
    });
    const select = environmentSelect(view.container)!;
    await act(async () => {
      select.value = "test";
      select.dispatchEvent(new Event("change", { bubbles: true }));
      await Promise.resolve();
    });

    expect(environmentSelect(view.container)!.disabled).toBe(false);
    expect(view.container.querySelector(".animate-spin")).toBeNull();
    view.unmount();
  });

  it("ignores a change to the environment that is already active", async () => {
    const view = mount(<OnboardingGate {...baseProps()} />);
    await act(async () => {
      await Promise.resolve();
    });
    const select = environmentSelect(view.container)!;
    act(() => {
      select.value = "production";
      select.dispatchEvent(new Event("change", { bubbles: true }));
    });
    expect(mocks.invoke).not.toHaveBeenCalled();
    view.unmount();
  });

  it("lists a custom placeholder when the environment is neither known value", async () => {
    mocks.env = { environment: "staging" };
    const view = mount(<OnboardingGate {...baseProps()} />);
    await act(async () => {
      await Promise.resolve();
    });
    const select = environmentSelect(view.container)!;
    expect(select.value).toBe("");
    expect([...select.options].some(option => option.value === "" && option.textContent === "custom")).toBe(true);
    view.unmount();
  });
});
