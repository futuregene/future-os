// @vitest-environment jsdom
import type { AgentModelOption } from "../../integrations/agent/agentClient";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ModelsPage } from "./ModelsPage";

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

const mocks = vi.hoisted(() => ({ listAgentProviders: vi.fn() }));

vi.mock("../../integrations/agent/providers", () => ({ listAgentProviders: mocks.listAgentProviders }));

const PROVIDERS_VIEW = {
  builtin: [
    { id: "future", name: "FutureOS", baseUrl: "", hasApiKey: true, modelCount: 1, requiresBaseUrl: false },
    { id: "deepseek", name: "DeepSeek", baseUrl: "", hasApiKey: false, modelCount: 1, requiresBaseUrl: false },
  ],
  custom: [{ id: "dashscope-coding", name: "DashScope", api: "openai", baseUrl: "https://x", hasApiKey: true, models: [] }],
};

function model(overrides: Partial<AgentModelOption> & Pick<AgentModelOption, "id" | "provider">): AgentModelOption {
  return { label: overrides.id, ...overrides };
}

const MODELS: AgentModelOption[] = [
  model({
    id: "deepseek-chat",
    label: "DeepSeek Chat",
    provider: "deepseek",
    supportsImages: false,
    description: "中文定位",
    descriptionEn: "   ",
  }),
  model({
    id: "gpt-5",
    label: "GPT-5",
    provider: "future",
    supportsImages: true,
    description: "未来平台说明",
    descriptionEn: "Frontier model for hard problems",
  }),
  model({ id: "qwen-max", label: "qwen-max", provider: "dashscope-coding" }),
];

const TWO_MODEL_PROVIDER = [
  model({ id: "a", label: "A", provider: "vendor" }),
  model({ id: "b", label: "B", provider: "vendor" }),
];

const roots: { root: ReturnType<typeof createRoot>; container: HTMLElement }[] = [];

beforeEach(() => {
  mocks.listAgentProviders.mockReset();
  mocks.listAgentProviders.mockResolvedValue(PROVIDERS_VIEW);
});

afterEach(() => {
  for (const { root, container } of roots.splice(0)) {
    act(() => root.unmount());
    container.remove();
  }
});

interface PageProps {
  hiddenModels?: string[];
  modelOptions?: AgentModelOption[];
  onChangeHidden?: (next: string[]) => void;
}

/** Mount and wait for the provider-name lookup, so sections show display names. */
async function renderPage(props: PageProps = {}) {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  roots.push({ container, root });
  await act(async () => {
    root.render(
      <ModelsPage
        hiddenModels={props.hiddenModels ?? []}
        modelOptions={props.modelOptions ?? MODELS}
        onChangeHidden={props.onChangeHidden ?? (() => {})}
      />,
    );
  });
  return container;
}

function switches(container: HTMLElement) {
  return [...container.querySelectorAll<HTMLButtonElement>("[role=switch]")];
}

function sectionTitles(container: HTMLElement) {
  return [...container.querySelectorAll("h3")].map(node => node.textContent);
}

function type(container: HTMLElement, value: string) {
  const input = container.querySelector<HTMLInputElement>("input")!;
  act(() => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, value);
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
}

describe("modelsPage layout", () => {
  it("groups models by provider display name", async () => {
    const container = await renderPage();

    expect(sectionTitles(container)).toEqual(["DashScope", "DeepSeek", "FutureOS"]);
  });

  it("falls back to the provider id when the agent does not name it", async () => {
    mocks.listAgentProviders.mockResolvedValue({ builtin: [], custom: [] });
    const container = await renderPage();

    expect(sectionTitles(container)).toEqual(["dashscope-coding", "deepseek", "future"]);
  });

  it("shows the curated description for the active language, falling back when it is blank", async () => {
    const container = await renderPage();

    // English UI: descriptionEn wins; a whitespace-only English blurb falls back
    // to the Chinese one instead of leaving the row blank.
    expect(container.textContent).toContain("中文定位");
    expect(container.textContent).toContain("Frontier model for hard problems");
  });

  it("renders the model id only when it differs from the label, plus the modality", async () => {
    const container = await renderPage();

    const rows = [...container.querySelectorAll("div")].map(node => node.textContent ?? "");
    // GPT-5: labelled "GPT-5" but keyed "gpt-5" ⇒ id shown before the modality.
    expect(rows.some(text => text.includes("gpt-5 · Text Image"))).toBe(true);
    expect(rows.some(text => text.includes("deepseek-chat · Text"))).toBe(true);
    // qwen-max's label equals its id, so only the modality remains.
    expect(rows.includes("Text")).toBe(true);
  });

  it("omits the description block entirely when neither variant is present", async () => {
    const container = await renderPage({ modelOptions: [model({ id: "bare", label: "Bare", provider: "vendor" })] });

    expect(container.textContent).toContain("Bare");
    expect(container.querySelector(".line-clamp-3")).toBeNull();
  });

  it("shows the empty state when the agent reports no models", async () => {
    const container = await renderPage({ modelOptions: [] });

    expect(container.textContent).toContain("No models loaded from the Agent");
    expect(sectionTitles(container)).toEqual([]);
  });

  it("shows the no-match state when a search filters everything out", async () => {
    const container = await renderPage();

    type(container, "gpt");
    expect(sectionTitles(container)).toEqual(["FutureOS"]);

    type(container, "zzzz");
    expect(container.textContent).toContain("No matching models.");
    expect(sectionTitles(container)).toEqual([]);
  });

  it.each([
    ["gpt", ["FutureOS"]], // model id / label
    ["GPT-5", ["FutureOS"]], // case-insensitive label
    ["deepseek", ["DeepSeek"]], // provider id and display name
    ["dashscope", ["DashScope"]], // provider id of a custom provider
    ["  QWEN  ", ["DashScope"]], // trimmed needle, id match
    ["", ["DashScope", "DeepSeek", "FutureOS"]], // empty needle ⇒ everything
    ["   ", ["DashScope", "DeepSeek", "FutureOS"]], // blank needle is not a filter
  ] as const)("filters on %j across label/id/provider/display-name", async (needle, expected) => {
    const container = await renderPage();

    type(container, needle);

    expect(sectionTitles(container)).toEqual([...expected]);
  });

  it("does not search the localized description", async () => {
    const container = await renderPage();

    type(container, "中文");

    expect(container.textContent).toContain("No matching models.");
  });
});

describe("modelsPage visibility", () => {
  it("hides one model by appending its provider/id key", async () => {
    const onChangeHidden = vi.fn();
    const container = await renderPage({ onChangeHidden });
    // Switch order per section: the provider switch first, then its models, and
    // sections are sorted by display name (DashScope, DeepSeek, FutureOS).
    const modelSwitch = switches(container)[3]!;

    expect(modelSwitch.getAttribute("aria-label")).toBe("DeepSeek Chat");
    expect(modelSwitch.getAttribute("aria-checked")).toBe("true");
    act(() => modelSwitch.click());

    expect(onChangeHidden).toHaveBeenCalledExactlyOnceWith(["deepseek/deepseek-chat"]);
  });

  it("shows a hidden model again without touching the other keys", async () => {
    const onChangeHidden = vi.fn();
    const container = await renderPage({ hiddenModels: ["deepseek/deepseek-chat", "future/gpt-5"], onChangeHidden });
    const modelSwitch = switches(container)[3]!;

    expect(modelSwitch.getAttribute("aria-checked")).toBe("false");
    act(() => modelSwitch.click());

    expect(onChangeHidden).toHaveBeenCalledExactlyOnceWith(["future/gpt-5"]);
  });

  it("uses the bare model id as the key when the model has no provider", async () => {
    const onChangeHidden = vi.fn();
    const container = await renderPage({
      modelOptions: [model({ id: "orphan", label: "Orphan", provider: "" })],
      onChangeHidden,
    });

    act(() => switches(container)[1]!.click());

    expect(onChangeHidden).toHaveBeenCalledExactlyOnceWith(["orphan"]);
  });

  it("hides every model of a provider in one patch", async () => {
    const onChangeHidden = vi.fn();
    const container = await renderPage({ modelOptions: TWO_MODEL_PROVIDER, onChangeHidden });
    const providerSwitch = switches(container)[0]!;

    expect(providerSwitch.getAttribute("aria-label")).toBe("vendor");
    expect(providerSwitch.getAttribute("aria-checked")).toBe("true");
    act(() => providerSwitch.click());

    expect(onChangeHidden).toHaveBeenCalledExactlyOnceWith(["vendor/a", "vendor/b"]);
  });

  it("appends only the provider's missing keys, leaving other providers alone", async () => {
    const onChangeHidden = vi.fn();
    const container = await renderPage({ modelOptions: TWO_MODEL_PROVIDER, hiddenModels: ["other/x"], onChangeHidden });
    const providerSwitch = switches(container)[0]!;

    expect(providerSwitch.getAttribute("aria-checked")).toBe("true");
    act(() => providerSwitch.click());

    expect(onChangeHidden).toHaveBeenCalledExactlyOnceWith(["other/x", "vendor/a", "vendor/b"]);
  });

  it("shows every model of a provider in one patch, dropping only its keys", async () => {
    const onChangeHidden = vi.fn();
    const container = await renderPage({
      modelOptions: TWO_MODEL_PROVIDER,
      hiddenModels: ["vendor/a", "vendor/b", "other/x"],
      onChangeHidden,
    });

    act(() => switches(container)[0]!.click());

    expect(onChangeHidden).toHaveBeenCalledExactlyOnceWith(["other/x"]);
  });

  it("labels every switch with its model or provider name for assistive tech", async () => {
    const container = await renderPage();

    const labels = switches(container).map(item => item.getAttribute("aria-label"));
    expect(labels).toEqual(["DashScope", "qwen-max", "DeepSeek", "DeepSeek Chat", "FutureOS", "GPT-5"]);
  });

  it("survives a very large model list (50 providers × 4 models)", async () => {
    const big: AgentModelOption[] = [];
    for (let p = 0; p < 50; p++) {
      for (let m = 0; m < 4; m++)
        big.push(model({ id: `m${m}`, label: `Model ${m}`, provider: `p${p}` }));
    }
    const container = await renderPage({ modelOptions: big });

    expect(sectionTitles(container)).toHaveLength(50);
    expect(switches(container)).toHaveLength(50 + 200);
  });
});
