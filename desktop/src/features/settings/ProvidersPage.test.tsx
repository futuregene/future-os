// @vitest-environment jsdom
import type { BuiltinProvider, CustomProvider, ProvidersView } from "../../integrations/agent/providers";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { onFutureEvent } from "../../lib/futureEvents";
import { ProvidersPage } from "./ProvidersPage";

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

const mocks = vi.hoisted(() => ({
  deleteCustomProvider: vi.fn(),
  listAgentProviders: vi.fn(),
  logoutFutureProvider: vi.fn(),
  peekAgentProviders: vi.fn(),
  syncFutureModels: vi.fn(),
  updateBuiltinProvider: vi.fn(),
  upsertCustomProvider: vi.fn(),
}));

vi.mock("../../integrations/agent/providers", () => ({
  deleteCustomProvider: mocks.deleteCustomProvider,
  listAgentProviders: mocks.listAgentProviders,
  logoutFutureProvider: mocks.logoutFutureProvider,
  peekAgentProviders: mocks.peekAgentProviders,
  updateBuiltinProvider: mocks.updateBuiltinProvider,
  upsertCustomProvider: mocks.upsertCustomProvider,
}));

vi.mock("../../integrations/agent/agentClient", () => ({ syncFutureModels: mocks.syncFutureModels }));

// The two dialogs are exercised by their own suites; here they are stubs that
// report the props the page handed them and let the test drive onSubmit with a
// known payload.
vi.mock("./BuiltinProviderKeyDialog", () => ({
  BuiltinProviderKeyDialog: (props: {
    onClose: () => void;
    onSubmit: (payload: { apiKey?: string | null; baseUrl?: string }) => Promise<void>;
    open: boolean;
    provider: BuiltinProvider | null;
  }) => (
    <div data-testid="key-dialog" data-open={String(props.open)} data-provider={props.provider?.id ?? ""}>
      <button
        data-testid="key-submit"
        onClick={() => void props.onSubmit({ apiKey: "sk-typed", baseUrl: "https://typed.example/v1" })}
      >
        key-submit
      </button>
      <button data-testid="key-clear" onClick={() => void props.onSubmit({ apiKey: null })}>key-clear</button>
      <button data-testid="key-close" onClick={props.onClose}>key-close</button>
    </div>
  ),
}));

vi.mock("./CustomProviderDialog", () => ({
  CustomProviderDialog: (props: {
    existing: { id: string }[];
    initial: CustomProvider | null;
    onClose: () => void;
    onSubmit: (input: unknown) => Promise<void>;
    open: boolean;
  }) => (
    <div
      data-testid="custom-dialog"
      data-existing={props.existing.map(item => item.id).join(",")}
      data-initial={props.initial?.id ?? "none"}
      data-open={String(props.open)}
    >
      <button
        data-testid="custom-submit"
        onClick={() => void props.onSubmit({ id: "newp", name: "New P", api: "openai", baseUrl: "https://x.test", models: [], create: true })}
      >
        custom-submit
      </button>
      <button data-testid="custom-close" onClick={props.onClose}>custom-close</button>
    </div>
  ),
}));

const FUTURE: BuiltinProvider = { id: "future", name: "FutureOS", baseUrl: "", hasApiKey: true, modelCount: 5, requiresBaseUrl: false };
const ANTHROPIC: BuiltinProvider = { id: "anthropic", name: "Anthropic", baseUrl: "", hasApiKey: false, modelCount: 3, requiresBaseUrl: false };
const AZURE: BuiltinProvider = {
  id: "azure-openai",
  name: "Azure OpenAI",
  baseUrl: "https://YOUR_RESOURCE.openai.azure.com",
  hasApiKey: true,
  modelCount: 2,
  requiresBaseUrl: true,
};

const CUSTOM: CustomProvider = {
  id: "dashscope-coding",
  name: "DashScope",
  api: "openai",
  baseUrl: "https://dashscope.aliyuncs.com/compatible-mode/v1",
  hasApiKey: true,
  models: [
    { id: "qwen", name: "Qwen", supportsImages: false, reasoning: true, contextWindow: 128000, maxTokens: 8192, inputCost: 0, outputCost: 0, cacheReadCost: 0, cacheWriteCost: 0 },
  ],
};

const VIEW: ProvidersView = { builtin: [FUTURE, ANTHROPIC, AZURE], custom: [] };
const EMPTY: ProvidersView = { builtin: [], custom: [] };

const roots: { root: ReturnType<typeof createRoot>; container: HTMLElement }[] = [];

beforeEach(() => {
  for (const key of Object.keys(mocks) as (keyof typeof mocks)[]) mocks[key].mockReset();
  mocks.peekAgentProviders.mockReturnValue(VIEW);
  mocks.listAgentProviders.mockResolvedValue(VIEW);
});

afterEach(() => {
  for (const { root, container } of roots.splice(0)) {
    act(() => root.unmount());
    container.remove();
  }
});

function button(container: HTMLElement, text: string) {
  return [...container.querySelectorAll("button")].find(item => item.textContent === text);
}

function testId(container: HTMLElement, id: string) {
  return container.querySelector<HTMLElement>(`[data-testid=${id}]`)!;
}

interface PageProps {
  communityEdition?: boolean;
  futureSessionStatus?: "signed_out" | "authenticated" | "invalid" | "unavailable" | "checking";
  onProvidersChanged?: () => void;
  onRefreshFutureAuth?: () => void;
}

async function renderPage(props: PageProps = {}) {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  roots.push({ container, root });
  await act(async () => {
    root.render(
      <ProvidersPage
        futureSessionStatus={props.futureSessionStatus ?? "signed_out"}
        onProvidersChanged={props.onProvidersChanged}
        onRefreshFutureAuth={props.onRefreshFutureAuth ?? (() => {})}
        communityEdition={props.communityEdition}
      />,
    );
  });
  return container;
}

describe("providersPage loading and errors", () => {
  it("renders nothing while the very first load is still in flight", async () => {
    mocks.peekAgentProviders.mockReturnValue(null);
    mocks.listAgentProviders.mockReturnValue(new Promise(() => {}));
    const container = await renderPage();

    expect(container.textContent).toBe("");
  });

  it("keeps rendering the cached view when the refetch rejects, and shows the error", async () => {
    mocks.listAgentProviders.mockRejectedValue(new Error("agent offline"));
    const container = await renderPage();

    expect(container.textContent).toContain("agent offline");
    // The cached view is still on screen — a failed poll must not blank it.
    expect(container.textContent).toContain("FutureOS");
    expect(container.textContent).toContain("Anthropic");
  });

  it("stringifies a non-Error load rejection", async () => {
    mocks.listAgentProviders.mockRejectedValue("plain failure");
    const container = await renderPage();

    expect(container.textContent).toContain("plain failure");
  });
});

describe("providersPage built-in catalogue", () => {
  it("hides non-default providers behind a count and toggles them", async () => {
    const container = await renderPage();

    expect(container.textContent).toContain("FutureOS");
    expect(container.textContent).toContain("Anthropic");
    expect(container.textContent).not.toContain("Azure OpenAI");

    act(() => button(container, "More providers (1)")!.click());
    expect(container.textContent).toContain("Azure OpenAI");

    act(() => button(container, "Hide providers")!.click());
    expect(container.textContent).not.toContain("Azure OpenAI");
  });

  it("renders an empty catalogue with no expand affordance", async () => {
    mocks.peekAgentProviders.mockReturnValue(EMPTY);
    mocks.listAgentProviders.mockResolvedValue(EMPTY);
    const container = await renderPage();

    expect(button(container, "More providers (1)")).toBeUndefined();
    expect(container.textContent).toContain("No custom providers yet.");
  });

  it("shows each built-in provider's model count, singular and plural", async () => {
    const single: ProvidersView = {
      builtin: [{ ...ANTHROPIC, id: "deepseek", name: "DeepSeek", modelCount: 1 }],
      custom: [],
    };
    mocks.peekAgentProviders.mockReturnValue(single);
    mocks.listAgentProviders.mockResolvedValue(single);
    const container = await renderPage();

    expect(container.textContent).toContain("1 model");
    expect(container.textContent).not.toContain("1 models");
  });

  it("labels configured and unconfigured providers", async () => {
    const view: ProvidersView = {
      builtin: [
        { ...ANTHROPIC, hasApiKey: true },
        { ...ANTHROPIC, id: "deepseek", name: "DeepSeek", hasApiKey: false },
      ],
      custom: [],
    };
    mocks.peekAgentProviders.mockReturnValue(view);
    mocks.listAgentProviders.mockResolvedValue(view);
    const container = await renderPage();

    expect(container.textContent).toContain("Configured");
    expect(container.textContent).toContain("Not configured");
  });
});

describe("providersPage Future account row", () => {
  it.each([
    ["authenticated", "Signed in"],
    ["invalid", "Sign-in expired"],
    ["unavailable", "Account temporarily unavailable"],
    ["checking", "Checking sign-in…"],
    ["signed_out", "Not signed in"],
  ] as const)("labels the %s session as %s", async (status, label) => {
    const container = await renderPage({ futureSessionStatus: status });

    expect(container.textContent).toContain(label);
  });

  it("offers connect for a signed-out account and asks for onboarding", async () => {
    const events: unknown[] = [];
    const off = onFutureEvent("show-onboarding", () => events.push("onboarding"));
    try {
      const container = await renderPage({ futureSessionStatus: "signed_out" });
      expect(button(container, "Update model list")).toBeUndefined();

      act(() => button(container, "Sign in")!.click());
      expect(events).toEqual(["onboarding"]);
    }
    finally {
      off();
    }
  });

  it("offers sign-in-again for an invalid session", async () => {
    const container = await renderPage({ futureSessionStatus: "invalid" });

    expect(button(container, "Sign in again")).toBeTruthy();
    expect(button(container, "Update model list")).toBeUndefined();
  });

  it("offers retry (not model sync) while the session check is unavailable", async () => {
    const onRefreshFutureAuth = vi.fn();
    const container = await renderPage({ futureSessionStatus: "unavailable", onRefreshFutureAuth });

    expect(button(container, "Update model list")).toBeUndefined();
    act(() => button(container, "Retry")!.click());
    expect(onRefreshFutureAuth).toHaveBeenCalledTimes(1);
  });

  it("disables model sync while the session is only being checked", async () => {
    const container = await renderPage({ futureSessionStatus: "checking" });

    const sync = button(container, "Update model list") as HTMLButtonElement;
    expect(sync.disabled).toBe(true);
  });
});

describe("providersPage model sync", () => {
  it("refreshes the catalogue, announces the sync and toasts the count", async () => {
    const onProvidersChanged = vi.fn();
    let resolveSync!: (value: { synced: boolean; modelCount: number }) => void;
    mocks.syncFutureModels.mockReturnValue(new Promise((resolve) => {
      resolveSync = resolve;
    }));
    const toasts: unknown[] = [];
    const syncs: unknown[] = [];
    const offToast = onFutureEvent("toast", detail => toasts.push(detail));
    const offSync = onFutureEvent("future-models-synced", () => syncs.push("synced"));

    try {
      const container = await renderPage({ futureSessionStatus: "authenticated", onProvidersChanged });
      const refreshed: ProvidersView = { builtin: [{ ...FUTURE, modelCount: 7 }], custom: [] };
      mocks.listAgentProviders.mockResolvedValue(refreshed);

      act(() => button(container, "Update model list")!.click());
      const busy = button(container, "Updating…") as HTMLButtonElement;
      expect(busy).toBeTruthy();
      expect(busy.disabled).toBe(true);

      await act(async () => {
        resolveSync({ synced: true, modelCount: 2 });
      });

      expect(mocks.listAgentProviders).toHaveBeenCalledTimes(2);
      expect(syncs).toEqual(["synced"]);
      expect(toasts).toEqual([{ message: "Updated 2 Future models.", tone: "info" }]);
      expect(onProvidersChanged).toHaveBeenCalledTimes(1);
      expect(container.textContent).toContain("7 models");
      expect(button(container, "Update model list")).toBeTruthy();
    }
    finally {
      offToast();
      offSync();
    }
  });

  it("treats a refused sync as an error and skips the catalogue refetch", async () => {
    mocks.syncFutureModels.mockResolvedValue({ synced: false, modelCount: 0 });
    const onProvidersChanged = vi.fn();
    const syncs: unknown[] = [];
    const offSync = onFutureEvent("future-models-synced", () => syncs.push("synced"));
    try {
      const container = await renderPage({ futureSessionStatus: "authenticated", onProvidersChanged });

      await act(async () => {
        button(container, "Update model list")!.click();
      });

      expect(container.textContent).toContain("Couldn't update the Future model list");
      expect(mocks.listAgentProviders).toHaveBeenCalledTimes(1);
      expect(syncs).toEqual([]);
      expect(onProvidersChanged).not.toHaveBeenCalled();
    }
    finally {
      offSync();
    }
  });

  it("reports a throwing sync and re-enables the button", async () => {
    mocks.syncFutureModels.mockRejectedValue(new Error("network down"));
    const container = await renderPage({ futureSessionStatus: "authenticated" });

    await act(async () => {
      button(container, "Update model list")!.click();
    });

    expect(container.textContent).toContain("network down");
    expect((button(container, "Update model list") as HTMLButtonElement).disabled).toBe(false);
  });
});

describe("providersPage sign-out", () => {
  it("asks for confirmation and applies the returned view", async () => {
    const onProvidersChanged = vi.fn();
    const signedOut: ProvidersView = { builtin: [{ ...FUTURE, hasApiKey: false, modelCount: 0 }], custom: [] };
    mocks.logoutFutureProvider.mockResolvedValue(signedOut);
    const container = await renderPage({ futureSessionStatus: "authenticated", onProvidersChanged });

    act(() => button(container, "Sign out")!.click());
    expect(container.textContent).toContain("Sign out?");

    act(() => button(container, "Cancel")!.click());
    expect(mocks.logoutFutureProvider).not.toHaveBeenCalled();
    expect(container.textContent).not.toContain("Sign out?");

    act(() => button(container, "Sign out")!.click());
    await act(async () => {
      button(container, "Sign out")!.click();
    });

    // The confirm button is the last "Sign out" in the row; the click above hit
    // the confirm one because the plain button was replaced by the confirm row.
    expect(mocks.logoutFutureProvider).toHaveBeenCalledTimes(1);
    expect(onProvidersChanged).toHaveBeenCalledTimes(1);
    expect(container.textContent).not.toContain("Sign out?");
    expect(container.textContent).toContain("0 models");
  });

  it("keeps the confirmation row and shows the failure when sign-out throws", async () => {
    mocks.logoutFutureProvider.mockRejectedValue(new Error("platform unreachable"));
    const container = await renderPage({ futureSessionStatus: "authenticated" });

    act(() => button(container, "Sign out")!.click());
    await act(async () => {
      button(container, "Sign out")!.click();
    });

    expect(container.textContent).toContain("platform unreachable");
    expect(container.textContent).toContain("Sign out?");
  });
});

describe("providersPage custom providers", () => {
  it("opens the add dialog with every known id and applies the upserted view", async () => {
    const onProvidersChanged = vi.fn();
    const withCustom: ProvidersView = { builtin: VIEW.builtin, custom: [CUSTOM] };
    mocks.upsertCustomProvider.mockResolvedValue(withCustom);
    const container = await renderPage({ onProvidersChanged });

    expect(testId(container, "custom-dialog").dataset.open).toBe("false");
    expect(testId(container, "custom-dialog").dataset.initial).toBe("none");

    act(() => button(container, "+ Add custom provider")!.click());
    expect(testId(container, "custom-dialog").dataset.open).toBe("true");
    expect(testId(container, "custom-dialog").dataset.existing).toBe("future,anthropic,azure-openai");

    await act(async () => {
      testId(container, "custom-submit").click();
    });

    expect(mocks.upsertCustomProvider).toHaveBeenCalledWith({
      id: "newp",
      name: "New P",
      api: "openai",
      baseUrl: "https://x.test",
      models: [],
      create: true,
    });
    expect(onProvidersChanged).toHaveBeenCalledTimes(1);
    expect(container.textContent).toContain("DashScope");
    expect(container.textContent).toContain("1 model");
  });

  it("closes the custom dialog without saving", async () => {
    const container = await renderPage();

    act(() => button(container, "+ Add custom provider")!.click());
    expect(testId(container, "custom-dialog").dataset.open).toBe("true");

    act(() => testId(container, "custom-close").click());
    expect(testId(container, "custom-dialog").dataset.open).toBe("false");
    expect(mocks.upsertCustomProvider).not.toHaveBeenCalled();
  });

  it("lists a custom provider's base URL and models count", async () => {
    const withCustom: ProvidersView = { builtin: VIEW.builtin, custom: [CUSTOM] };
    mocks.peekAgentProviders.mockReturnValue(withCustom);
    mocks.listAgentProviders.mockResolvedValue(withCustom);
    const container = await renderPage();

    expect(container.textContent).toContain("https://dashscope.aliyuncs.com/compatible-mode/v1");
    expect(container.textContent).not.toContain("No custom providers yet.");
  });

  it("opens the edit dialog with the row as initial value", async () => {
    const withCustom: ProvidersView = { builtin: VIEW.builtin, custom: [CUSTOM] };
    mocks.peekAgentProviders.mockReturnValue(withCustom);
    mocks.listAgentProviders.mockResolvedValue(withCustom);
    const container = await renderPage();

    act(() => button(container, "Edit")!.click());
    expect(testId(container, "custom-dialog").dataset.open).toBe("true");
    expect(testId(container, "custom-dialog").dataset.initial).toBe("dashscope-coding");
  });

  it("removes a custom provider only after confirmation", async () => {
    const withCustom: ProvidersView = { builtin: VIEW.builtin, custom: [CUSTOM] };
    mocks.peekAgentProviders.mockReturnValue(withCustom);
    mocks.listAgentProviders.mockResolvedValue(withCustom);
    mocks.deleteCustomProvider.mockResolvedValue(VIEW);
    const onProvidersChanged = vi.fn();
    const container = await renderPage({ onProvidersChanged });

    act(() => button(container, "Remove")!.click());
    expect(container.textContent).toContain("Remove?");
    expect(mocks.deleteCustomProvider).not.toHaveBeenCalled();

    await act(async () => {
      // The confirm row's button is the only "Remove" still rendered.
      button(container, "Remove")!.click();
    });

    expect(mocks.deleteCustomProvider).toHaveBeenCalledWith("dashscope-coding");
    expect(onProvidersChanged).toHaveBeenCalledTimes(1);
    expect(container.textContent).toContain("No custom providers yet.");
  });

  it("cancels a pending delete without touching the provider", async () => {
    const withCustom: ProvidersView = { builtin: VIEW.builtin, custom: [CUSTOM] };
    mocks.peekAgentProviders.mockReturnValue(withCustom);
    mocks.listAgentProviders.mockResolvedValue(withCustom);
    const container = await renderPage();

    act(() => button(container, "Remove")!.click());
    expect(container.textContent).toContain("Remove?");

    act(() => button(container, "Cancel")!.click());
    expect(container.textContent).not.toContain("Remove?");
    expect(button(container, "Edit")).toBeTruthy();
    expect(mocks.deleteCustomProvider).not.toHaveBeenCalled();
  });

  it("keeps the confirmation row and reports a failed delete", async () => {
    const withCustom: ProvidersView = { builtin: VIEW.builtin, custom: [CUSTOM] };
    mocks.peekAgentProviders.mockReturnValue(withCustom);
    mocks.listAgentProviders.mockResolvedValue(withCustom);
    mocks.deleteCustomProvider.mockRejectedValue(new Error("provider in use"));
    const container = await renderPage();

    act(() => button(container, "Remove")!.click());
    await act(async () => {
      button(container, "Remove")!.click();
    });

    expect(container.textContent).toContain("provider in use");
    expect(container.textContent).toContain("Remove?");
    expect(container.textContent).toContain("DashScope");
  });
});

describe("providersPage built-in keys", () => {
  it("saves a typed key and reports it by provider name", async () => {
    mocks.updateBuiltinProvider.mockResolvedValue(VIEW);
    const onProvidersChanged = vi.fn();
    const container = await renderPage({ onProvidersChanged });

    act(() => button(container, "Configure")!.click());
    expect(testId(container, "key-dialog").dataset.provider).toBe("anthropic");

    await act(async () => {
      testId(container, "key-submit").click();
    });

    expect(mocks.updateBuiltinProvider).toHaveBeenCalledWith({
      apiKey: "sk-typed",
      baseUrl: "https://typed.example/v1",
      id: "anthropic",
      updateApiKey: true,
    });
    expect(onProvidersChanged).toHaveBeenCalledTimes(1);
    expect(container.textContent).toContain("Saved the Anthropic key. It takes effect immediately.");
    expect(testId(container, "key-dialog").dataset.open).toBe("false");
  });

  it("clears a stored key and reports the clear", async () => {
    mocks.updateBuiltinProvider.mockResolvedValue(VIEW);
    const container = await renderPage();

    act(() => button(container, "Configure")!.click());
    await act(async () => {
      testId(container, "key-clear").click();
    });

    expect(mocks.updateBuiltinProvider).toHaveBeenCalledWith({
      apiKey: null,
      baseUrl: undefined,
      id: "anthropic",
      updateApiKey: true,
    });
    expect(container.textContent).toContain("Cleared the Anthropic key.");
  });

  it("closes the key dialog without saving", async () => {
    const container = await renderPage();

    act(() => button(container, "Configure")!.click());
    act(() => testId(container, "key-close").click());

    expect(testId(container, "key-dialog").dataset.open).toBe("false");
    expect(mocks.updateBuiltinProvider).not.toHaveBeenCalled();
  });

  it("uses the same key flow for Future in the community edition", async () => {
    mocks.updateBuiltinProvider.mockResolvedValue(VIEW);
    const container = await renderPage({ communityEdition: true, futureSessionStatus: "authenticated" });

    // No account controls in community edition.
    expect(button(container, "Update model list")).toBeUndefined();
    expect(button(container, "Sign out")).toBeUndefined();
    const configure = [...container.querySelectorAll("button")].filter(item => item.textContent === "Configure");
    expect(configure).toHaveLength(2); // future + anthropic

    act(() => configure[0]!.click());
    expect(testId(container, "key-dialog").dataset.provider).toBe("future");

    await act(async () => {
      testId(container, "key-submit").click();
    });
    expect(mocks.updateBuiltinProvider).toHaveBeenCalledWith(expect.objectContaining({ id: "future" }));
  });

  it("offers the base-URL flow for a placeholder provider", async () => {
    mocks.updateBuiltinProvider.mockResolvedValue(VIEW);
    const container = await renderPage();

    act(() => button(container, "More providers (1)")!.click());
    const configure = [...container.querySelectorAll("button")].filter(item => item.textContent === "Configure");
    expect(configure).toHaveLength(2);

    act(() => configure[1]!.click());
    expect(testId(container, "key-dialog").dataset.provider).toBe("azure-openai");
  });
});

describe("providersPage catalogue reloads", () => {
  it("refetches when the agent reports a provider change", async () => {
    const container = await renderPage();
    expect(mocks.listAgentProviders).toHaveBeenCalledTimes(1);

    const updated: ProvidersView = { builtin: [FUTURE], custom: [] };
    mocks.listAgentProviders.mockResolvedValue(updated);
    await act(async () => {
      window.dispatchEvent(new CustomEvent("futureos:providers-changed", { detail: { revision: 1, providerId: "future", operation: "update", authChanged: false, modelsChanged: true } }));
    });

    expect(mocks.listAgentProviders).toHaveBeenCalledTimes(2);
    expect(container.textContent).not.toContain("Anthropic");
  });
});
