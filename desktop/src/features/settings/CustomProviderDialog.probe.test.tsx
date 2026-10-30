// @vitest-environment jsdom
import type { CustomProvider, CustomProviderModel, ThinkingLevelProbeInput, ThinkingLevelProbeOutcome } from "../../integrations/agent/providers";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import { CustomProviderDialog } from "./CustomProviderDialog";

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

/** The known wire values in their fixed display order — what detection tries. */
const KNOWN = ["off", "minimal", "low", "medium", "high", "xhigh", "max"];

function model(patch: Partial<CustomProviderModel> = {}): CustomProviderModel {
  return {
    id: "qwen",
    name: "",
    supportsImages: false,
    reasoning: true,
    thinkingLevels: [],
    contextWindow: 128000,
    maxTokens: 16384,
    inputCost: 0,
    outputCost: 0,
    cacheReadCost: 0,
    cacheWriteCost: 0,
    ...patch,
  };
}

function provider(models: CustomProviderModel[]): CustomProvider {
  return {
    id: "acme",
    name: "Acme",
    api: "openai-completions",
    baseUrl: "https://api.acme.test/v1",
    hasApiKey: true,
    models,
  };
}

let root: ReturnType<typeof createRoot>;
afterEach(() => {
  act(() => root.unmount());
  document.body.innerHTML = "";
});

function button(text: string) {
  const found = [...document.querySelectorAll("button")].find(button => button.textContent === text);
  expect(found, `button "${text}"`).toBeTruthy();
  return found!;
}

function maybeButton(text: string) {
  return [...document.querySelectorAll("button")].find(button => button.textContent === text);
}

function pressedLabels(): string[] {
  return [...document.querySelectorAll<HTMLButtonElement>("button[aria-pressed]")]
    .filter(chip => chip.getAttribute("aria-pressed") === "true")
    .map(chip => chip.textContent ?? "");
}

function mount(initial: CustomProvider, onProbe?: (input: ThinkingLevelProbeInput) => Promise<ThinkingLevelProbeOutcome>) {
  const container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
  act(() => root.render(
    <CustomProviderDialog
      existing={[]}
      initial={initial}
      onClose={vi.fn()}
      onProbe={onProbe}
      onSubmit={vi.fn().mockResolvedValue(undefined)}
      open
    />,
  ));
}

/** Answers every probe: `default` and the levels in `accepted` pass. */
function accepting(accepted: string[]) {
  return vi.fn(async (input: ThinkingLevelProbeInput): Promise<ThinkingLevelProbeOutcome> => {
    const level = input.level ?? "default";
    return level === "default" || accepted.includes(level)
      ? { level, ok: true }
      : { level, ok: false, error: `unsupported ${level}` };
  });
}

describe("custom provider thinking-level detection", () => {
  it("has no detect control when the provider is not reachable from here", () => {
    mount(provider([model({ thinkingLevels: ["low"] })]));
    expect(maybeButton("Detect")).toBeUndefined();
  });

  it("tests the default request first, then every level, and selects the accepted ones", async () => {
    const onProbe = accepting(["low", "high"]);
    mount(provider([model({ thinkingLevels: ["off", "medium"] })]), onProbe);
    expect(pressedLabels()).toEqual(["Off", "Medium"]);

    await act(async () => button("Detect").click());

    // The calibration call comes first and sends no level at all, so a wrong
    // Base URL / key / model id fails before any level is judged.
    const levels = onProbe.mock.calls.map(([input]) => (input.level ?? "default"));
    expect(levels).toEqual(["default", ...KNOWN]);
    expect(onProbe.mock.calls[0]?.[0]).toMatchObject({
      api: "openai-completions",
      baseUrl: "https://api.acme.test/v1",
      modelId: "qwen",
      providerId: "acme",
    });
    // Exactly the accepted levels replace the previous guess.
    expect(pressedLabels()).toEqual(["Low", "High"]);
    expect(document.body.textContent).toContain("Selected the 2 accepted level(s).");
  });

  it("stops at a failing default request and leaves the levels untouched", async () => {
    const onProbe = vi.fn(async (): Promise<ThinkingLevelProbeOutcome> => ({
      error: "Authentication failed (HTTP 401): invalid api key",
      level: "default",
      ok: false,
    }));
    mount(provider([model({ thinkingLevels: ["low"] })]), onProbe);
    await act(async () => button("Detect").click());

    expect(onProbe).toHaveBeenCalledTimes(1);
    expect(pressedLabels()).toEqual(["Low"]);
    expect(document.body.textContent).toContain("The default request failed");
    expect(document.body.textContent).toContain("invalid api key");
  });

  it("shrinks the selection to default when nothing is accepted", async () => {
    mount(provider([model({ thinkingLevels: ["low", "high"] })]), accepting([]));
    await act(async () => button("Detect").click());

    expect(pressedLabels()).toEqual([]);
    expect(document.body.textContent).toContain("No level was accepted");
  });

  it("reports progress per level while it runs", async () => {
    const pending: Array<() => void> = [];
    const onProbe = vi.fn((input: ThinkingLevelProbeInput) => new Promise<ThinkingLevelProbeOutcome>((resolve) => {
      pending.push(() => resolve({ level: input.level ?? "default", ok: true }));
    }));
    mount(provider([model({ thinkingLevels: [] })]), onProbe);

    act(() => button("Detect").click());
    expect(document.body.textContent).toContain("Testing Default…");

    await act(async () => pending.shift()!());
    expect(document.body.textContent).toContain("Testing Off…");

    // Drain the remaining levels one round trip at a time so the run finishes
    // before unmount.
    for (let answered = 0; answered <= KNOWN.length && pending.length > 0; answered += 1)
      await act(async () => pending.shift()!());
    expect(document.body.textContent).toContain(`Selected the ${KNOWN.length} accepted level(s).`);
  });

  it("keeps the levels already proven when a later probe cannot be sent", async () => {
    let calls = 0;
    const onProbe = vi.fn(async (input: ThinkingLevelProbeInput): Promise<ThinkingLevelProbeOutcome> => {
      calls += 1;
      if (calls > 2)
        throw new Error("Future Agent is unavailable");
      const level = input.level ?? "default";
      return { level, ok: level === "default" || level === "off" };
    });
    mount(provider([model({ thinkingLevels: ["medium"] })]), onProbe);
    await act(async () => button("Detect").click());

    // `default` and `off` were answered, so `off` is selected; the transport
    // failure is reported rather than silently dropping what was proven.
    expect(pressedLabels()).toEqual(["Off"]);
    expect(document.body.textContent).toContain("Detection failed");
    expect(document.body.textContent).toContain("Future Agent is unavailable");
  });

  it("asks for the Base URL and model ID before probing an incomplete row", async () => {
    const onProbe = accepting(["low"]);
    mount({ ...provider([model({ id: "" })]), baseUrl: "" }, onProbe);
    await act(async () => maybeButton("Detect")!.click());

    expect(onProbe).not.toHaveBeenCalled();
    expect(document.body.textContent).toContain("Fill in the Base URL and the model ID first.");
  });

  it("stops probing once the dialog is closed", async () => {
    const pending: Array<() => void> = [];
    const onProbe = vi.fn((input: ThinkingLevelProbeInput) => new Promise<ThinkingLevelProbeOutcome>((resolve) => {
      pending.push(() => resolve({ level: input.level ?? "default", ok: true }));
    }));
    const container = document.createElement("div");
    document.body.append(container);
    root = createRoot(container);
    const render = (open: boolean) => act(() => root.render(
      <CustomProviderDialog
        existing={[]}
        initial={provider([model({ thinkingLevels: [] })])}
        onClose={vi.fn()}
        onProbe={onProbe}
        onSubmit={vi.fn().mockResolvedValue(undefined)}
        open={open}
      />,
    ));
    render(true);

    act(() => button("Detect").click());
    await act(async () => pending.shift()!());
    expect(onProbe).toHaveBeenCalledTimes(2);

    // Closing abandons the run: the level already on the wire is not followed
    // by another one, so a dismissed dialog stops spending model calls.
    render(false);
    for (let answered = 0; answered < 3 && pending.length > 0; answered += 1)
      await act(async () => pending.shift()!());
    expect(onProbe).toHaveBeenCalledTimes(2);
  });
});
