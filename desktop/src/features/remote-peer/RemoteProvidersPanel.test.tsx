// @vitest-environment jsdom
import type { ProvidersView } from "../../integrations/agent/providers";
import type { RemotePeer } from "./remotePeerClient";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { beforeEach, expect, it, vi } from "vitest";
import { RemoteProvidersPanel } from "./RemoteProvidersPanel";

/**
 * Another computer's providers and keys.
 *
 * What matters here is **where a write lands and what shape it has**: a key is a
 * credential, and a write that clears one by accident locks that machine out of
 * its models. The two dialogs are this app's own and are covered where they
 * live; the panel supplies the writes they call.
 *
 * Tested against the Tauri boundary rather than a stubbed helper, so the wire
 * shape below is the shape that actually leaves the app.
 */

const invoke = vi.hoisted(() => vi.fn());
vi.mock("../../integrations/tauri/invoke", () => ({ invokeCommand: invoke }));

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

function hostView(overrides: Partial<ProvidersView> = {}): ProvidersView {
  return {
    builtin: [
      { baseUrl: "", hasApiKey: true, id: "future", modelCount: 5, name: "Future", requiresBaseUrl: false },
      { baseUrl: "", hasApiKey: true, id: "openai", modelCount: 4, name: "OpenAI", requiresBaseUrl: false },
      { baseUrl: "", hasApiKey: false, id: "deepseek", modelCount: 3, name: "DeepSeek", requiresBaseUrl: false },
    ],
    custom: [{
      api: "openai-completions",
      baseUrl: "https://example.test",
      hasApiKey: true,
      id: "mine",
      models: [{
        id: "m",
        name: "M",
        supportsImages: false,
        reasoning: true,
        contextWindow: 1,
        maxTokens: 1,
        inputCost: 0,
        outputCost: 0,
        cacheReadCost: 0,
        cacheWriteCost: 0,
      }],
      name: "Mine",
    }],
    ...overrides,
  };
}

function peer(overrides: Partial<RemotePeer> = {}): RemotePeer {
  return {
    agentAvailable: true,
    bridgeInstanceId: "b",
    connected: true,
    desktopId: "desktop_a",
    error: null,
    features: ["provider_management_v1"],
    icon: "laptop",
    name: "Studio iMac",
    pairId: "pair_1",
    ...overrides,
  };
}

/** The command that reached the backend, whichever helper sent it. */
function sent(type: string) {
  return invoke.mock.calls
    .map(([, args]) => args as { command?: Record<string, unknown> })
    .filter(args => args?.command?.type === type);
}

async function mount(props: { available?: boolean; peer?: RemotePeer } = {}) {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  await act(async () => {
    root.render(
      <RemoteProvidersPanel
        available={props.available ?? true}
        peer={props.peer ?? peer()}
      />,
    );
  });
  await settle();
  return {
    container,
    button: (text: string) =>
      [...container.querySelectorAll("button")].find(node => node.textContent === text),
    text: () => container.textContent ?? "",
    unmount: async () => {
      await act(async () => root.unmount());
      container.remove();
    },
  };
}

async function settle(): Promise<void> {
  await act(async () => {
    for (let i = 0; i < 8; i += 1)
      await Promise.resolve();
  });
}

async function type(input: HTMLInputElement, value: string) {
  await act(async () => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, value);
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
}

beforeEach(() => {
  document.body.innerHTML = "";
  invoke.mockReset();
  invoke.mockImplementation(async (_command, args) => {
    const type = (args as { command?: { type?: string } })?.command?.type;
    if (type === "list_providers")
      return hostView();
    if (type === "update_builtin_provider" || type === "upsert_custom_provider" || type === "delete_custom_provider")
      return hostView();
    return undefined;
  });
});

/** The host's own view, read from it rather than from this app's cache. */
it("reads that host's providers", async () => {
  const panel = await mount();

  expect(sent("list_providers")[0]).toEqual({
    desktopId: "desktop_a",
    command: { type: "list_providers" },
    lane: "list",
  });
  expect(panel.text()).toContain("Mine");
  expect(panel.text()).toContain("DeepSeek");
  await panel.unmount();
});

/**
 * The account provider is signed in on that computer, and the host refuses to
 * edit it — so it is shown rather than offered.
 */
it("shows the account provider without offering to edit it", async () => {
  const panel = await mount();

  expect(panel.text()).toContain("Signed in on that computer");
  // The row for it carries no action of its own, while another built-in that
  // holds a key does.
  const rows = [...panel.container.querySelectorAll("div")];
  const futureRow = rows.find(row => row.textContent === "Future" || row.textContent?.startsWith("FutureSigned"))!;
  expect(futureRow.querySelector("button")).toBeNull();
  expect(panel.button("Change key")).toBeTruthy();
  await panel.unmount();
});

/** A built-in key write goes to that host, with the host's own field names. */
it("writes a built-in key to that host", async () => {
  const panel = await mount();
  await act(async () => panel.button("Configure")!.click());
  await settle();

  await type(panel.container.querySelector<HTMLInputElement>("input")!, "sk-123");
  await act(async () => panel.button("Save")!.click());
  await settle();

  expect(sent("update_builtin_provider")[0]).toEqual({
    desktopId: "desktop_a",
    command: {
      type: "update_builtin_provider",
      provider: { apiKey: "sk-123", baseUrl: undefined, id: "deepseek", updateApiKey: true },
    },
    lane: "list",
  });
  await panel.unmount();
});

/**
 * Clearing a key has to say so.
 *
 * The host reads an absent key as "leave it", so a clear that sent nothing would
 * silently keep the old key — while the page reported it cleared.
 */
it("clears a key explicitly rather than by omission", async () => {
  const panel = await mount();
  await act(async () => panel.button("Change key")!.click());
  await settle();

  await act(async () => panel.button("Clear key")!.click());
  await settle();

  expect(sent("update_builtin_provider")[0]!.command).toEqual({
    type: "update_builtin_provider",
    // `null` is "clear it"; the same write with the key absent would keep it.
    provider: { apiKey: null, baseUrl: undefined, id: "openai", updateApiKey: true },
  });
  await panel.unmount();
});

/** Adding a custom provider writes through the dialog's own shape. */
it("adds a custom provider on that host", async () => {
  const panel = await mount();
  await act(async () => panel.button("+ Add custom provider")!.click());
  await settle();

  const fields = [...panel.container.querySelectorAll<HTMLInputElement>("input")];
  await type(fields[0]!, "newone");
  await type(fields[1]!, "New One");
  await type(fields[2]!, "https://new.test");
  await act(async () => panel.button("Save")!.click());
  await settle();

  expect(sent("upsert_custom_provider")[0]!.command).toMatchObject({
    type: "upsert_custom_provider",
    provider: { id: "newone", name: "New One", baseUrl: "https://new.test", create: true },
  });
  await panel.unmount();
});

/** Editing an existing custom provider writes the same shape with `create: false`. */
it("edits an existing custom provider in place", async () => {
  const panel = await mount();
  await act(async () => panel.button("Edit")!.click());
  await settle();

  // The dialog opens on that provider, not on an empty form.
  const fields = [...panel.container.querySelectorAll<HTMLInputElement>("input")];
  expect(fields[0]!.value).toBe("mine");
  expect(fields[1]!.value).toBe("Mine");

  await type(fields[1]!, "Mine Renamed");
  await act(async () => panel.button("Save")!.click());
  await settle();

  const upsert = sent("upsert_custom_provider")[0]!.command as { provider: Record<string, unknown> };
  expect(upsert.provider).toMatchObject({ id: "mine", name: "Mine Renamed", create: false });
  await panel.unmount();
});

/** Closing the key dialog without saving writes nothing. */
it("writes nothing when the key dialog is dismissed", async () => {
  const panel = await mount();
  await act(async () => panel.button("Change key")!.click());
  await settle();

  await act(async () => panel.button("Cancel")!.click());
  await settle();

  expect(sent("update_builtin_provider")).toEqual([]);
  await panel.unmount();
});

/** Removing a custom provider asks first, then removes it there. */
it("removes a custom provider after confirming", async () => {
  const panel = await mount();
  await act(async () => panel.button("Remove")!.click());
  await settle();

  expect(panel.text()).toContain("Remove this provider from");
  await act(async () => panel.button("Remove?")!.click());
  await settle();

  expect(sent("delete_custom_provider")[0]!.command).toEqual({
    type: "delete_custom_provider",
    providerId: "mine",
  });
  await panel.unmount();
});

/** Backing out of the confirmation writes nothing. */
it("writes nothing when a removal is cancelled", async () => {
  const panel = await mount();
  await act(async () => panel.button("Remove")!.click());
  await settle();

  await act(async () => panel.button("Cancel")!.click());
  await settle();

  expect(sent("delete_custom_provider")).toEqual([]);
  await panel.unmount();
});

/** A host the user is not connected to cannot be asked or told anything. */
it("closes every action while the host is unreachable", async () => {
  const panel = await mount({ available: false, peer: peer({ connected: false }) });

  for (const node of panel.container.querySelectorAll<HTMLButtonElement>("button"))
    expect(node.disabled).toBe(true);
  await panel.unmount();
});

/** A refused write is reported, and the page is not left looking as if it worked. */
it("surfaces a refused write", async () => {
  invoke.mockImplementation(async (_command, args) => {
    const type = (args as { command?: { type?: string } })?.command?.type;
    if (type === "list_providers")
      return hostView();
    throw new Error("invalid api key");
  });
  const panel = await mount();
  await act(async () => panel.button("Configure")!.click());
  await settle();

  await type(panel.container.querySelector<HTMLInputElement>("input")!, "bad");
  await act(async () => panel.button("Save")!.click());
  await settle();

  expect(panel.text()).toContain("invalid api key");
  await panel.unmount();
});

/** A failed read is a page-level error, not a silently empty list. */
it("reports a failed read", async () => {
  invoke.mockRejectedValue(new Error("peer_not_connected"));
  const panel = await mount();

  expect(panel.text()).toContain("peer_not_connected");
  await panel.unmount();
});
