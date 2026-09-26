// @vitest-environment jsdom
import type { BuiltinProvider } from "../../integrations/agent/providers";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import { BuiltinProviderKeyDialog } from "./BuiltinProviderKeyDialog";

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

const KEYED: BuiltinProvider = {
  id: "anthropic",
  name: "Anthropic",
  baseUrl: "https://api.anthropic.com",
  hasApiKey: true,
  modelCount: 3,
  requiresBaseUrl: false,
};

const UNKEYED: BuiltinProvider = { ...KEYED, hasApiKey: false };

const PLACEHOLDER_PROVIDER: BuiltinProvider = {
  id: "azure-openai",
  name: "Azure OpenAI",
  baseUrl: "https://YOUR_RESOURCE.openai.azure.com",
  hasApiKey: true,
  modelCount: 2,
  requiresBaseUrl: true,
};

const CONFIGURED_AZURE: BuiltinProvider = {
  ...PLACEHOLDER_PROVIDER,
  baseUrl: "https://my-resource.openai.azure.com",
};

const roots: { root: ReturnType<typeof createRoot>; container: HTMLElement }[] = [];

afterEach(() => {
  for (const { root, container } of roots.splice(0)) {
    act(() => root.unmount());
    container.remove();
  }
});

function mount(provider: BuiltinProvider | null, onSubmit: (payload: unknown) => Promise<void>, onClose = vi.fn()) {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  roots.push({ container, root });
  act(() => {
    root.render(
      <BuiltinProviderKeyDialog onClose={onClose} onSubmit={onSubmit} open provider={provider} />,
    );
  });
  return container;
}

function button(container: HTMLElement, text: string) {
  return [...container.querySelectorAll("button")].find(item => item.textContent === text) as HTMLButtonElement | undefined;
}

/** The primary footer button, whatever the in-flight label currently is. */
function saveButton(container: HTMLElement) {
  const found = [...container.querySelectorAll("button")].find(
    item => item.textContent === "Save" || item.textContent === "Saving…",
  );
  expect(found).toBeTruthy();
  return found as HTMLButtonElement;
}

function inputs(container: HTMLElement) {
  return [...container.querySelectorAll("input")];
}

function type(input: HTMLInputElement, value: string) {
  act(() => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, value);
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
}

describe("builtinProviderKeyDialog key entry", () => {
  it("trims the key, reports the in-flight save and closes via onSubmit's caller", async () => {
    let resolveSubmit!: () => void;
    const onSubmit = vi.fn(() => new Promise<void>((resolve) => {
      resolveSubmit = resolve;
    }));
    const container = mount(KEYED, onSubmit);
    const key = inputs(container)[0]!;
    type(key, "  sk-anthropic-key  ");

    await act(async () => {
      button(container, "Save")!.click();
    });

    expect(onSubmit).toHaveBeenCalledWith({ apiKey: "sk-anthropic-key" });
    expect(button(container, "Saving…")!.disabled).toBe(true);
    expect(button(container, "Clear key")!.disabled).toBe(true);
    expect(button(container, "Cancel")!.disabled).toBe(false);

    await act(async () => {
      resolveSubmit();
    });
    // The dialog stays in its saving state until the parent closes it.
    expect(button(container, "Saving…")).toBeTruthy();
  });

  it.each([
    ["", "Please enter an API key."],
    ["   ", "Please enter an API key."],
    ["\t\n", "Please enter an API key."],
  ])("rejects the blank key %j before calling the backend", async (value, message) => {
    const onSubmit = vi.fn().mockResolvedValue(undefined);
    const container = mount(KEYED, onSubmit);

    type(inputs(container)[0]!, value);
    await act(async () => {
      button(container, "Save")!.click();
    });

    expect(container.textContent).toContain(message);
    expect(onSubmit).not.toHaveBeenCalled();
  });

  it.each([
    ["sk-plain", "sk-plain"],
    ["  sk-padded  ", "sk-padded"],
    ["键-值-😀", "键-值-😀"],
    [`sk-${"x".repeat(4000)}`, `sk-${"x".repeat(4000)}`],
  ])("forwards %j as %j (trimmed, no other rewriting)", async (typed, expected) => {
    const onSubmit = vi.fn().mockResolvedValue(undefined);
    const container = mount(UNKEYED, onSubmit);

    type(inputs(container)[0]!, typed);
    await act(async () => {
      button(container, "Save")!.click();
    });

    expect(onSubmit).toHaveBeenCalledWith({ apiKey: expected });
  });

  it("clears a stored key without asking for a new one", async () => {
    const onSubmit = vi.fn().mockResolvedValue(undefined);
    const container = mount(KEYED, onSubmit);

    await act(async () => {
      button(container, "Clear key")!.click();
    });

    expect(onSubmit).toHaveBeenCalledWith({ apiKey: null });
  });

  it("offers no clear button when no key is stored", () => {
    const container = mount(UNKEYED, vi.fn());

    expect(button(container, "Clear key")).toBeUndefined();
  });

  it("shows the backend failure and re-enables both buttons", async () => {
    const onSubmit = vi.fn().mockRejectedValue(new Error("agent rejected the key"));
    const container = mount(KEYED, onSubmit);
    type(inputs(container)[0]!, "sk-x");

    await act(async () => {
      button(container, "Save")!.click();
    });

    expect(container.textContent).toContain("agent rejected the key");
    expect(button(container, "Save")!.disabled).toBe(false);
    expect(button(container, "Clear key")!.disabled).toBe(false);
  });

  it("stringifies a non-Error save rejection and clears the message on retry", async () => {
    const onSubmit = vi.fn().mockRejectedValueOnce("socket closed").mockResolvedValueOnce(undefined);
    const container = mount(KEYED, onSubmit);
    type(inputs(container)[0]!, "sk-x");

    await act(async () => {
      button(container, "Save")!.click();
    });
    expect(container.textContent).toContain("socket closed");

    await act(async () => {
      button(container, "Save")!.click();
    });
    expect(container.textContent).not.toContain("socket closed");
    expect(onSubmit).toHaveBeenCalledTimes(2);
  });

  it("closes through Cancel and through the backdrop", () => {
    const onClose = vi.fn();
    const container = mount(KEYED, vi.fn(), onClose);

    act(() => button(container, "Cancel")!.click());
    expect(onClose).toHaveBeenCalledTimes(1);

    act(() => container.querySelector<HTMLButtonElement>("button[aria-label=Close]")!.click());
    expect(onClose).toHaveBeenCalledTimes(2);
  });

  it("keeps no typed key when the dialog is closed and reopened", async () => {
    const onSubmit = vi.fn().mockResolvedValue(undefined);
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    roots.push({ container, root });
    const view = (open: boolean) => (
      <BuiltinProviderKeyDialog onClose={vi.fn()} onSubmit={onSubmit} open={open} provider={KEYED} />
    );

    act(() => root.render(view(true)));
    type(inputs(container)[0]!, "sk-typed-once");
    act(() => root.render(view(false)));
    expect(container.textContent).toBe("");
    act(() => root.render(view(true)));

    expect(inputs(container)[0]!.value).toBe("");
    await act(async () => {
      button(container, "Save")!.click();
    });
    expect(onSubmit).not.toHaveBeenCalled();
    expect(container.textContent).toContain("Please enter an API key.");
  });

  it("renders the provider name in the title and tolerates a null provider", () => {
    const container = mount(KEYED, vi.fn());
    expect(container.querySelector("h2")!.textContent).toBe("Set Anthropic key");
    expect(container.textContent).toContain("The key is stored only on this computer");

    const empty = mount(null, vi.fn());
    expect(empty.querySelector("h2")!.textContent).toBe("Set  key");
  });
});

describe("builtinProviderKeyDialog Base URL flow", () => {
  it("asks for the Base URL before saving anything", async () => {
    const onSubmit = vi.fn().mockResolvedValue(undefined);
    const container = mount(PLACEHOLDER_PROVIDER, onSubmit);

    expect(container.textContent).toContain("This provider's Base URL is a placeholder");
    expect(inputs(container)[0]!.placeholder).toBe("https://YOUR_RESOURCE.openai.azure.com");
    expect(inputs(container)[1]!.placeholder).toBe("Leave blank to keep the current key");

    await act(async () => {
      saveButton(container).click();
    });
    expect(container.textContent).toContain("Please enter a Base URL.");
    expect(onSubmit).not.toHaveBeenCalled();
  });

  it("forwards a trimmed Base URL and leaves a blank key untouched", async () => {
    const onSubmit = vi.fn().mockResolvedValue(undefined);
    const container = mount(PLACEHOLDER_PROVIDER, onSubmit);

    type(inputs(container)[0]!, "  https://my-resource.openai.azure.com/v1  ");
    await act(async () => {
      saveButton(container).click();
    });

    // No `apiKey` field at all ⇒ the stored key is untouched.
    expect(onSubmit).toHaveBeenCalledExactlyOnceWith({ baseUrl: "https://my-resource.openai.azure.com/v1" });
  });

  it("forwards the key too when the user types one", async () => {
    const onSubmit = vi.fn().mockResolvedValue(undefined);
    const container = mount(PLACEHOLDER_PROVIDER, onSubmit);

    type(inputs(container)[0]!, "https://my-resource.openai.azure.com/v1");
    type(inputs(container)[1]!, "  sk-azure  ");
    await act(async () => {
      saveButton(container).click();
    });

    expect(onSubmit).toHaveBeenCalledExactlyOnceWith({
      apiKey: "sk-azure",
      baseUrl: "https://my-resource.openai.azure.com/v1",
    });
  });

  it("leaves the unfilled placeholder out of the prefilled value", () => {
    const container = mount(PLACEHOLDER_PROVIDER, vi.fn());

    expect(inputs(container)[0]!.value).toBe("");
  });

  it("prefills an already configured Base URL", () => {
    const container = mount(CONFIGURED_AZURE, vi.fn());

    expect(inputs(container)[0]!.value).toBe("https://my-resource.openai.azure.com");
  });

  it("still reports a rejected Base URL save", async () => {
    const onSubmit = vi.fn().mockRejectedValue(new Error("bad url"));
    const container = mount(CONFIGURED_AZURE, onSubmit);
    type(inputs(container)[0]!, "https://nope.test");

    await act(async () => {
      button(container, "Save")!.click();
    });

    expect(container.textContent).toContain("bad url");
  });
});
