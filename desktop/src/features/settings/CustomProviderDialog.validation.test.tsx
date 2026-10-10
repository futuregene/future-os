// @vitest-environment jsdom
import type { CustomProvider, CustomProviderModel } from "../../integrations/agent/providers";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { CustomProviderDialog } from "./CustomProviderDialog";

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

const BLANK_MODEL: CustomProviderModel = {
  id: "",
  name: "",
  supportsImages: false,
  reasoning: true,
  contextWindow: 128000,
  maxTokens: 16384,
  inputCost: 0,
  outputCost: 0,
  cacheReadCost: 0,
  cacheWriteCost: 0,
};

const EXISTING = [
  { id: "future", name: "FutureOS" },
  { id: "dashscope-coding", name: "DashScope" },
];

const EDITING: CustomProvider = {
  id: "dashscope-coding",
  name: "DashScope",
  api: "anthropic",
  baseUrl: "https://dashscope.example.com/v1",
  hasApiKey: true,
  models: [{ ...BLANK_MODEL, id: "qwen", name: "Qwen", contextWindow: 200000, maxTokens: 8192, inputCost: 0.5, outputCost: 1.5, cacheReadCost: 0.1, cacheWriteCost: 0.2 }],
};

const roots: { root: ReturnType<typeof createRoot>; container: HTMLElement }[] = [];

beforeEach(() => {
  let seed = 0;
  vi.spyOn(crypto, "randomUUID").mockImplementation(() => `00000000-0000-4000-8000-${String(seed++).padStart(12, "0")}` as `${string}-${string}-${string}-${string}-${string}`);
});

afterEach(() => {
  vi.restoreAllMocks();
  for (const { root, container } of roots.splice(0)) {
    act(() => root.unmount());
    container.remove();
  }
});

interface MountOptions {
  existing?: { id: string; name: string }[];
  initial?: CustomProvider | null;
  onClose?: () => void;
  onSubmit?: (input: unknown) => Promise<void>;
  open?: boolean;
}

function mount(options: MountOptions = {}) {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  roots.push({ container, root });
  const onSubmit = options.onSubmit ?? vi.fn().mockResolvedValue(undefined);
  act(() => {
    root.render(
      <CustomProviderDialog
        existing={options.existing ?? EXISTING}
        initial={options.initial ?? null}
        onClose={options.onClose ?? vi.fn()}
        onSubmit={onSubmit}
        open={options.open ?? true}
      />,
    );
  });
  return container;
}

function byText(container: HTMLElement, text: string) {
  return [...container.querySelectorAll("button")].find(item => item.textContent === text) as HTMLButtonElement;
}

function errorText(container: HTMLElement) {
  return [...container.querySelectorAll("p")].find(node => node.className.includes("text-danger"))?.textContent ?? null;
}

/** Plain `<TextInput>` fields (id, name, baseUrl, then each model's id/name). */
function textInputs(container: HTMLElement) {
  return [...container.querySelectorAll<HTMLInputElement>("input:not([type])")];
}

function idInput(container: HTMLElement) {
  return textInputs(container)[0]!;
}

function nameInput(container: HTMLElement) {
  return textInputs(container)[1]!;
}

function baseUrlInput(container: HTMLElement) {
  return textInputs(container)[2]!;
}

function apiKeyInput(container: HTMLElement) {
  return container.querySelector<HTMLInputElement>("input[type=password]")!;
}

function apiSelect(container: HTMLElement) {
  return container.querySelector<HTMLSelectElement>("select")!;
}

/** Numeric fields of one model row, in DOM order. */
function numberInputs(container: HTMLElement, row = 0) {
  const rows = [...container.querySelectorAll<HTMLInputElement>("input[type=number]")];
  return rows.slice(row * 6, row * 6 + 6);
}

function modelIdInput(container: HTMLElement, row = 0) {
  return textInputs(container)[3 + row * 2]!;
}

function modelNameInput(container: HTMLElement, row = 0) {
  return textInputs(container)[4 + row * 2]!;
}

function addModel(container: HTMLElement) {
  act(() => byText(container, "+ Add model").click());
}

function setInput(input: HTMLInputElement | HTMLSelectElement, value: string) {
  act(() => {
    const proto = input instanceof HTMLSelectElement ? HTMLSelectElement.prototype : HTMLInputElement.prototype;
    Object.getOwnPropertyDescriptor(proto, "value")!.set!.call(input, value);
    input.dispatchEvent(new Event(input instanceof HTMLSelectElement ? "change" : "input", { bubbles: true }));
  });
}

function clickCheckbox(container: HTMLElement, row: number, name: "reasoning" | "image") {
  const boxes = [...container.querySelectorAll<HTMLInputElement>("input[type=checkbox]")];
  // Per row: reasoning, the disabled text box, then the image box.
  const offset = row * 3 + (name === "reasoning" ? 0 : 2);
  act(() => boxes[offset]!.click());
}

/** Fill the provider fields needed to get past the provider-level checks. */
function fillProvider(container: HTMLElement, overrides: { id?: string; name?: string; baseUrl?: string; apiKey?: string } = {}) {
  setInput(idInput(container), overrides.id ?? "my-provider");
  setInput(nameInput(container), overrides.name ?? "My Provider");
  setInput(baseUrlInput(container), overrides.baseUrl ?? "https://api.example.com/v1");
  if (overrides.apiKey !== undefined)
    setInput(apiKeyInput(container), overrides.apiKey);
}

describe("customProviderDialog provider validation", () => {
  it("requires a provider id", () => {
    const onSubmit = vi.fn();
    const container = mount({ onSubmit });

    act(() => byText(container, "Save").click());

    expect(errorText(container)).toBe("Please enter a provider ID.");
    expect(onSubmit).not.toHaveBeenCalled();
  });

  it("treats a whitespace-only id as missing", () => {
    const container = mount();

    setInput(idInput(container), "   ");
    act(() => byText(container, "Save").click());

    expect(errorText(container)).toBe("Please enter a provider ID.");
  });

  it.each([
    ["a", "Provider ID must be between 2 and 40 characters."],
    ["x".repeat(41), "Provider ID must be between 2 and 40 characters."],
  ])("bounds the id length (%j)", (id, message) => {
    const container = mount();

    setInput(idInput(container), id);
    act(() => byText(container, "Save").click());

    expect(errorText(container)).toBe(message);
  });

  it("accepts the boundary id lengths 2 and 40", async () => {
    const onSubmit = vi.fn().mockResolvedValue(undefined);
    const container = mount({ onSubmit });

    // 40 chars is the inclusive maximum.
    fillProvider(container, { id: "z".repeat(40) });
    await act(async () => {
      byText(container, "Save").click();
    });
    expect(onSubmit).toHaveBeenCalledExactlyOnceWith(expect.objectContaining({ id: "z".repeat(40) }));
  });

  it("lowercases the id and rejects characters outside [a-z0-9_-]", () => {
    const container = mount();

    setInput(idInput(container), "MY PROVIDER!");
    // The field lowercases on change; the remaining space/! still fail the pattern.
    expect(idInput(container).value).toBe("my provider!");
    act(() => byText(container, "Save").click());

    expect(errorText(container)).toBe("Provider ID may only contain lowercase letters, digits, '-' and '_'.");
  });

  it("rejects an id that already exists", () => {
    const container = mount();

    setInput(idInput(container), "future");
    act(() => byText(container, "Save").click());

    expect(errorText(container)).toBe("That provider ID already exists, please choose another.");
  });

  it.each([
    ["", "Please enter a Base URL."],
    ["   ", "Please enter a Base URL."],
    ["not a url", "Base URL must be a valid http/https address."],
    ["ftp://files.example.com", "Base URL must be a valid http/https address."],
    ["file:///etc/passwd", "Base URL must be a valid http/https address."],
  ])("rejects baseUrl %j", (baseUrl, message) => {
    const container = mount();

    fillProvider(container, { baseUrl });
    act(() => byText(container, "Save").click());

    expect(errorText(container)).toBe(message);
  });

  it.each([
    "https://api.example.com/v1",
    "http://127.0.0.1:8080",
    "https://api.example.com/v1/",
  ])("accepts the http(s) baseUrl %j", async (baseUrl) => {
    const onSubmit = vi.fn().mockResolvedValue(undefined);
    const container = mount({ onSubmit });

    fillProvider(container, { baseUrl });
    await act(async () => {
      byText(container, "Save").click();
    });

    expect(onSubmit).toHaveBeenCalledExactlyOnceWith(expect.objectContaining({ baseUrl }));
  });

  it("bounds the provider name", () => {
    const container = mount();

    fillProvider(container, { name: "n".repeat(41) });
    act(() => byText(container, "Save").click());
    expect(errorText(container)).toBe("Provider name cannot exceed 40 characters.");
  });

  it.each(["名字", "Provider 😀", "Provider！"])("rejects the non-ASCII name %j", (name) => {
    const container = mount();

    fillProvider(container, { name });
    act(() => byText(container, "Save").click());

    expect(errorText(container)).toBe("Provider name may only contain letters, digits, spaces and _.()-; Chinese / emoji / full-width characters are not supported.");
  });

  it("rejects a name another provider already uses, case-insensitively", () => {
    const container = mount();

    fillProvider(container, { name: "dashscope" });
    act(() => byText(container, "Save").click());

    expect(errorText(container)).toBe("That provider name already exists, please choose another.");
  });

  it("allows the name to be left empty (the backend falls back to the id)", async () => {
    const onSubmit = vi.fn().mockResolvedValue(undefined);
    const container = mount({ onSubmit });

    fillProvider(container, { name: "" });
    await act(async () => {
      byText(container, "Save").click();
    });

    expect(onSubmit).toHaveBeenCalledExactlyOnceWith(expect.objectContaining({ name: "" }));
  });

  it("accepts the documented name characters", async () => {
    const onSubmit = vi.fn().mockResolvedValue(undefined);
    const container = mount({ onSubmit });

    fillProvider(container, { name: "Acme (self-hosted).v2-beta_1" });
    await act(async () => {
      byText(container, "Save").click();
    });

    expect(onSubmit).toHaveBeenCalledExactlyOnceWith(expect.objectContaining({ name: "Acme (self-hosted).v2-beta_1" }));
  });
});

describe("customProviderDialog model validation", () => {
  function withModel(container: HTMLElement, id = "qwen") {
    fillProvider(container);
    addModel(container);
    setInput(modelIdInput(container), id);
  }

  it("bounds the model id length", () => {
    const container = mount();
    withModel(container, "m".repeat(101));

    act(() => byText(container, "Save").click());

    expect(errorText(container)).toBe(`Model ID "${"m".repeat(101)}" is too long.`);
  });

  it("accepts the boundary model id length 100", async () => {
    const onSubmit = vi.fn().mockResolvedValue(undefined);
    const container = mount({ onSubmit });
    withModel(container, "m".repeat(100));

    await act(async () => {
      byText(container, "Save").click();
    });

    expect(onSubmit).toHaveBeenCalledExactlyOnceWith(expect.objectContaining({
      models: [expect.objectContaining({ id: "m".repeat(100) })],
    }));
  });

  it.each(["qwen v2", "qwen#1", "qwen@"])("rejects the model id %j", (id) => {
    const container = mount();
    withModel(container, id);

    act(() => byText(container, "Save").click());

    expect(errorText(container)).toBe(`Model ID "${id}" contains invalid characters.`);
  });

  it.each(["qwen", "qwen-max", "vendor/model:1.5", "a_b-c"])("accepts the model id %j", async (id) => {
    const onSubmit = vi.fn().mockResolvedValue(undefined);
    const container = mount({ onSubmit });
    withModel(container, id);

    await act(async () => {
      byText(container, "Save").click();
    });

    expect(onSubmit).toHaveBeenCalledExactlyOnceWith(expect.objectContaining({
      models: [expect.objectContaining({ id })],
    }));
  });

  it("rejects a duplicated model id", () => {
    const container = mount();
    fillProvider(container);
    addModel(container);
    setInput(modelIdInput(container, 0), "qwen");
    addModel(container);
    setInput(modelIdInput(container, 1), "qwen");

    act(() => byText(container, "Save").click());

    expect(errorText(container)).toBe("Model ID \"qwen\" is duplicated.");
  });

  it("treats ids that differ only by surrounding spaces as duplicates", () => {
    const container = mount();
    fillProvider(container);
    addModel(container);
    setInput(modelIdInput(container, 0), "qwen");
    addModel(container);
    setInput(modelIdInput(container, 1), " qwen ");

    act(() => byText(container, "Save").click());

    expect(errorText(container)).toBe("Model ID \"qwen\" is duplicated.");
  });

  it("bounds the model display name", () => {
    const container = mount();
    withModel(container);
    setInput(modelNameInput(container), "n".repeat(61));

    act(() => byText(container, "Save").click());

    expect(errorText(container)).toBe(`Model name "${"n".repeat(61)}" is too long.`);
  });

  it("drops rows whose id was left empty instead of failing", async () => {
    const onSubmit = vi.fn().mockResolvedValue(undefined);
    const container = mount({ onSubmit });
    fillProvider(container);
    addModel(container); // left blank

    await act(async () => {
      byText(container, "Save").click();
    });

    expect(onSubmit).toHaveBeenCalledExactlyOnceWith(expect.objectContaining({ models: [] }));
  });

  it.each([0, -1, 1.5])("requires a positive integer context window (%j)", (contextWindow) => {
    const container = mount();
    withModel(container);
    setInput(numberInputs(container)[0]!, String(contextWindow));

    act(() => byText(container, "Save").click());

    expect(errorText(container)).toBe("Token limits for model \"qwen\" must be positive integers.");
  });

  it.each([0, -5, 2.5])("requires a positive integer max tokens (%j)", (maxTokens) => {
    const container = mount();
    withModel(container);
    setInput(numberInputs(container)[1]!, String(maxTokens));

    act(() => byText(container, "Save").click());

    expect(errorText(container)).toBe("Token limits for model \"qwen\" must be positive integers.");
  });

  it("rejects max tokens above the context window", () => {
    const container = mount();
    withModel(container);
    setInput(numberInputs(container)[0]!, "1000");
    setInput(numberInputs(container)[1]!, "1001");

    act(() => byText(container, "Save").click());

    expect(errorText(container)).toBe("Maximum output tokens for model \"qwen\" cannot exceed its context window.");
  });

  it("accepts max tokens equal to the context window", async () => {
    const onSubmit = vi.fn().mockResolvedValue(undefined);
    const container = mount({ onSubmit });
    withModel(container);
    setInput(numberInputs(container)[0]!, "4096");
    setInput(numberInputs(container)[1]!, "4096");

    await act(async () => {
      byText(container, "Save").click();
    });

    expect(onSubmit).toHaveBeenCalledExactlyOnceWith(expect.objectContaining({
      models: [expect.objectContaining({ contextWindow: 4096, maxTokens: 4096 })],
    }));
  });

  it.each([0, 1, 2, 3])("rejects a negative price in slot %d", (slot) => {
    const container = mount();
    withModel(container);
    setInput(numberInputs(container)[2 + slot]!, "-0.5");

    act(() => byText(container, "Save").click());

    expect(errorText(container)).toBe("Prices for model \"qwen\" must be non-negative numbers.");
  });

  it("accepts fractional and zero prices, forwarding them verbatim", async () => {
    const onSubmit = vi.fn().mockResolvedValue(undefined);
    const container = mount({ onSubmit });
    withModel(container);
    setInput(numberInputs(container)[2]!, "0.000125");
    setInput(numberInputs(container)[3]!, "1.5");
    setInput(numberInputs(container)[4]!, "0.25");
    setInput(numberInputs(container)[5]!, "0");

    await act(async () => {
      byText(container, "Save").click();
    });

    expect(onSubmit).toHaveBeenCalledExactlyOnceWith(expect.objectContaining({
      models: [expect.objectContaining({ inputCost: 0.000125, outputCost: 1.5, cacheReadCost: 0.25, cacheWriteCost: 0 })],
    }));
  });

  it("forwards the modality and thinking flags from the row", async () => {
    const onSubmit = vi.fn().mockResolvedValue(undefined);
    const container = mount({ onSubmit });
    withModel(container);
    clickCheckbox(container, 0, "image");
    clickCheckbox(container, 0, "reasoning");

    await act(async () => {
      byText(container, "Save").click();
    });

    expect(onSubmit).toHaveBeenCalledExactlyOnceWith(expect.objectContaining({
      models: [expect.objectContaining({ supportsImages: true, reasoning: false })],
    }));
  });

  it("removes a row with its trash button", async () => {
    const onSubmit = vi.fn().mockResolvedValue(undefined);
    const container = mount({ onSubmit });
    fillProvider(container);
    addModel(container);
    setInput(modelIdInput(container, 0), "keep");
    addModel(container);
    setInput(modelIdInput(container, 1), "drop");
    expect(container.querySelectorAll("input[type=number]")).toHaveLength(12);

    act(() => container.querySelectorAll<HTMLButtonElement>("button[aria-label='Remove model']")[1]!.click());
    expect(container.querySelectorAll("input[type=number]")).toHaveLength(6);

    await act(async () => {
      byText(container, "Save").click();
    });

    expect(onSubmit).toHaveBeenCalledExactlyOnceWith(expect.objectContaining({
      models: [expect.objectContaining({ id: "keep" })],
    }));
  });

  it("shows the empty-models hint until a row is added", () => {
    const container = mount();

    expect(container.textContent).toContain("No models added yet.");
    addModel(container);
    expect(container.textContent).not.toContain("No models added yet.");
  });

  /**
   * Model-count boundaries. The rows are seeded through `initial` (the edit
   * path) rather than 100 UI clicks: the limit is enforced over the submitted
   * model list, and clicking "+ Add model" 100 times only exercises React's
   * reconciler (it made the suite take minutes under coverage instrumentation).
   */
  function manyModels(count: number) {
    return Array.from({ length: count }, (_, index) => ({
      ...BLANK_MODEL,
      id: `m${index}`,
      name: `Model ${index}`,
    }));
  }

  it("accepts exactly 100 models", async () => {
    const onSubmit = vi.fn().mockResolvedValue(undefined);
    const container = mount({ initial: { ...EDITING, models: manyModels(100) }, onSubmit });

    await act(async () => {
      byText(container, "Save").click();
    });

    expect(errorText(container)).toBeNull();
    expect(onSubmit).toHaveBeenCalledTimes(1);
    expect((onSubmit.mock.calls[0]![0] as { models: unknown[] }).models).toHaveLength(100);
    // Rendering a 100-row form is inherently slow; the default 5s budget is not
    // a behavioural bound here.
  }, 60_000);

  it("rejects 101 models with the model-count error", async () => {
    const onSubmit = vi.fn().mockResolvedValue(undefined);
    const container = mount({ initial: { ...EDITING, models: manyModels(101) }, onSubmit });

    await act(async () => {
      byText(container, "Save").click();
    });

    expect(errorText(container)).toBe("The number of models cannot exceed 100.");
    expect(onSubmit).not.toHaveBeenCalled();
  }, 60_000);
});

describe("customProviderDialog submit and lifecycle", () => {
  it("sends the trimmed key, the api type and create=true for a new provider", async () => {
    const onSubmit = vi.fn().mockResolvedValue(undefined);
    const container = mount({ onSubmit });
    fillProvider(container, { apiKey: "  sk-secret  " });
    setInput(apiSelect(container), "openai-responses");

    await act(async () => {
      byText(container, "Save").click();
    });

    expect(onSubmit).toHaveBeenCalledExactlyOnceWith({
      api: "openai-responses",
      apiKey: "sk-secret",
      baseUrl: "https://api.example.com/v1",
      create: true,
      id: "my-provider",
      models: [],
      name: "My Provider",
    });
  });

  it("sends a null key when the field is left blank", async () => {
    const onSubmit = vi.fn().mockResolvedValue(undefined);
    const container = mount({ onSubmit });
    fillProvider(container, { apiKey: "   " });

    await act(async () => {
      byText(container, "Save").click();
    });

    expect(onSubmit).toHaveBeenCalledExactlyOnceWith(expect.objectContaining({ apiKey: null }));
  });

  it("shows the in-flight state and closes only after the parent resolves", async () => {
    let resolveSubmit!: () => void;
    const onSubmit = vi.fn(() => new Promise<void>((resolve) => {
      resolveSubmit = resolve;
    }));
    const onClose = vi.fn();
    const container = mount({ onClose, onSubmit });
    fillProvider(container);

    act(() => byText(container, "Save").click());
    expect(byText(container, "Saving…").disabled).toBe(true);
    expect(onClose).not.toHaveBeenCalled();

    await act(async () => {
      resolveSubmit();
    });
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("keeps the dialog open and reports an Error rejection", async () => {
    const onSubmit = vi.fn().mockRejectedValue(new Error("provider id is reserved"));
    const onClose = vi.fn();
    const container = mount({ onClose, onSubmit });
    fillProvider(container);

    await act(async () => {
      byText(container, "Save").click();
    });

    expect(errorText(container)).toBe("provider id is reserved");
    expect(onClose).not.toHaveBeenCalled();
    expect(byText(container, "Save").disabled).toBe(false);
  });

  it("stringifies a non-Error rejection and clears it on retry", async () => {
    const onSubmit = vi.fn().mockRejectedValueOnce("ipc closed").mockResolvedValueOnce(undefined);
    const container = mount({ onSubmit });
    fillProvider(container);

    await act(async () => {
      byText(container, "Save").click();
    });
    expect(errorText(container)).toBe("ipc closed");

    await act(async () => {
      byText(container, "Save").click();
    });
    expect(errorText(container)).toBeNull();
    expect(onSubmit).toHaveBeenCalledTimes(2);
  });

  it("cancels without submitting", () => {
    const onSubmit = vi.fn();
    const onClose = vi.fn();
    const container = mount({ onClose, onSubmit });

    act(() => byText(container, "Cancel").click());

    expect(onClose).toHaveBeenCalledTimes(1);
    expect(onSubmit).not.toHaveBeenCalled();
  });

  it("renders nothing when closed, and resets the form on reopen", () => {
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    roots.push({ container, root });
    const view = (open: boolean) => (
      <CustomProviderDialog existing={EXISTING} initial={null} onClose={vi.fn()} onSubmit={vi.fn().mockResolvedValue(undefined)} open={open} />
    );

    act(() => root.render(view(false)));
    expect(container.textContent).toBe("");

    act(() => root.render(view(true)));
    fillProvider(container, { id: "first", apiKey: "sk-1" });
    addModel(container);
    setInput(modelIdInput(container), "row");
    expect(container.textContent).not.toContain("No models added yet.");

    act(() => root.render(view(false)));
    act(() => root.render(view(true)));

    expect(idInput(container).value).toBe("");
    expect(nameInput(container).value).toBe("");
    expect(baseUrlInput(container).value).toBe("");
    expect(apiKeyInput(container).value).toBe("");
    expect(apiSelect(container).value).toBe("openai-completions");
    expect(container.textContent).toContain("No models added yet.");
  });
});

describe("customProviderDialog editing", () => {
  it("prefills the stored provider with the id locked", () => {
    const container = mount({ initial: EDITING });

    expect(container.querySelector("h2")!.textContent).toBe("Edit custom provider");
    expect(idInput(container).value).toBe("dashscope-coding");
    expect(idInput(container).disabled).toBe(true);
    expect(nameInput(container).value).toBe("DashScope");
    expect(baseUrlInput(container).value).toBe("https://dashscope.example.com/v1");
    expect(apiSelect(container).value).toBe("anthropic");
    expect(apiKeyInput(container).value).toBe("");
    expect(container.textContent).toContain("API key (leave blank to keep current)");
    expect(modelIdInput(container).value).toBe("qwen");
    expect(modelNameInput(container).value).toBe("Qwen");
    const context = numberInputs(container)[0]!;
    const max = numberInputs(container)[1]!;
    expect(context.value).toBe("200000");
    expect(max.value).toBe("8192");
    expect(container.textContent).not.toContain("Test build");
  });

  it("submits create=false, keeping the id and sending the untouched key as null", async () => {
    const onSubmit = vi.fn().mockResolvedValue(undefined);
    const container = mount({ initial: EDITING, onSubmit });

    await act(async () => {
      byText(container, "Save").click();
    });

    expect(onSubmit).toHaveBeenCalledExactlyOnceWith({
      api: "anthropic",
      apiKey: null,
      baseUrl: "https://dashscope.example.com/v1",
      create: false,
      id: "dashscope-coding",
      models: [expect.objectContaining({ id: "qwen", name: "Qwen", contextWindow: 200000, maxTokens: 8192, inputCost: 0.5, cacheWriteCost: 0.2 })],
      name: "DashScope",
    });
  });

  it("allows the provider to keep its own name", async () => {
    const onSubmit = vi.fn().mockResolvedValue(undefined);
    const container = mount({ initial: EDITING, onSubmit });

    await act(async () => {
      byText(container, "Save").click();
    });

    expect(errorText(container)).toBeNull();
    expect(onSubmit).toHaveBeenCalledTimes(1);
  });

  it("still rejects a name another provider already uses", () => {
    const container = mount({ initial: EDITING });

    setInput(nameInput(container), "FutureOS");
    act(() => byText(container, "Save").click());

    expect(errorText(container)).toBe("That provider name already exists, please choose another.");
  });

  it("falls back to the OpenAI Completions api when the stored value is empty", async () => {
    const onSubmit = vi.fn().mockResolvedValue(undefined);
    const container = mount({ initial: { ...EDITING, api: "" }, onSubmit });

    expect(apiSelect(container).value).toBe("openai-completions");
    await act(async () => {
      byText(container, "Save").click();
    });
    expect(onSubmit).toHaveBeenCalledExactlyOnceWith(expect.objectContaining({ api: "openai-completions" }));
  });

  it("renders the price inputs as blank for zero prices and as the number otherwise", () => {
    const container = mount({ initial: EDITING });
    const prices = numberInputs(container).slice(2);

    expect(prices.map(input => input.value)).toEqual(["0.5", "1.5", "0.1", "0.2"]);

    const zeroPriced = mount({ initial: { ...EDITING, models: [{ ...BLANK_MODEL, id: "free" }] } });
    expect(numberInputs(zeroPriced).slice(2).map(input => input.value)).toEqual(["", "", "", ""]);
  });

  it("keeps the price input uncontrolled so the typed text is never reformatted", async () => {
    const onSubmit = vi.fn().mockResolvedValue(undefined);
    const container = mount({ initial: { ...EDITING, models: [{ ...BLANK_MODEL, id: "qwen" }] }, onSubmit });
    const input = numberInputs(container)[2]!;

    setInput(input, "1.5");
    // Nothing writes back a parsed/formatted number (a controlled input would
    // re-render from state, so a fractional intermediate could be lost).
    expect(input.value).toBe("1.5");
    setInput(input, "0.000125");
    expect(input.value).toBe("0.000125");

    // ...and the fractional price reaches the submit payload unchanged.
    await act(async () => {
      byText(container, "Save").click();
    });
    expect(onSubmit).toHaveBeenCalledExactlyOnceWith(expect.objectContaining({
      models: [expect.objectContaining({ inputCost: 0.000125 })],
    }));
  });
});
