// @vitest-environment jsdom
import type { RemotePeer } from "./remotePeerClient";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { beforeEach, expect, it, vi } from "vitest";
import { RemoteModelsPanel } from "./RemoteModelsPanel";

/**
 * A machine's models, grouped by provider, with visibility switches.
 *
 * Two things here are easy to get wrong and neither is visible as an error: the
 * reference* a switch writes (a bare model id, or one under the wrong provider,
 * hides a different model on that machine), and which switch a click writes
 * (turning a provider off must not save only the rows currently rendered).
 */

const listRemoteSettingsModels = vi.fn<(desktopId: string) => Promise<unknown[]>>();
vi.mock("./remotePeerClient", async (importOriginal) => {
  const actual = await importOriginal<typeof import("./remotePeerClient")>();
  return {
    ...actual,
    listRemoteSettingsModels: (...args: Parameters<typeof listRemoteSettingsModels>) => listRemoteSettingsModels(...args),
  };
});

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

const peer: RemotePeer = {
  agentAvailable: true,
  bridgeInstanceId: "bridge_1",
  connected: true,
  stopped: false,
  desktopId: "desktop_a",
  error: null,
  features: [],
  icon: "laptop",
  name: "Studio iMac",
  pairId: "pair_1",
};

async function mount(props: { hidden?: string[]; online?: boolean } = {}) {
  const onChangeHidden = vi.fn();
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  await act(async () => {
    root.render(
      <RemoteModelsPanel
        available={props.online ?? true}
        hidden={props.hidden ?? []}
        onChangeHidden={onChangeHidden}
        peer={peer}
      />,
    );
  });
  await settle();
  return {
    container,
    onChangeHidden,
    search: async (value: string) => {
      await act(async () => {
        const input = container.querySelector("input")!;
        const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!;
        setter.call(input, value);
        input.dispatchEvent(new Event("input", { bubbles: true }));
      });
    },
    switchFor: (label: string) => container.querySelector<HTMLButtonElement>(`button[aria-label="${label}"]`),
    foldFor: (label: string) => [...container.querySelectorAll("button")]
      .find(node => node.textContent?.startsWith(label)),
    text: () => container.textContent ?? "",
    unmount: async () => {
      await act(async () => root.unmount());
      container.remove();
    },
  };
}

async function settle(): Promise<void> {
  await act(async () => {
    for (let i = 0; i < 6; i += 1) await Promise.resolve();
  });
}

beforeEach(() => {
  document.body.innerHTML = "";
  listRemoteSettingsModels.mockReset().mockResolvedValue([
    { id: "gpt-5", label: "GPT-5", provider: "openai" },
    { id: "o3", provider: "openai" },
    { id: "claude-opus", label: "Opus", provider: "anthropic" },
    // A model with no provider: its reference is the bare id.
    { id: "local-llama" },
  ]);
});

it("groups the host's models by provider, and counts them", async () => {
  const view = await mount();

  expect(view.text()).toContain("openai");
  expect(view.text()).toContain("anthropic");
  expect(view.text()).toContain("GPT-5");
  // A model with no label falls back to its id, and one with no provider is its
  // own group rather than being dropped.
  expect(view.text()).toContain("o3");
  expect(view.text()).toContain("local-llama");
  await view.unmount();
});

/**
 * The switch writes the *reference* (`provider/id`), which is what the host
 * resolves. The bare id under the wrong provider is a different model.
 */
it("writes the provider-qualified reference for a model", async () => {
  const view = await mount();

  await act(async () => view.switchFor("Opus")!.click());

  expect(view.onChangeHidden).toHaveBeenCalledWith(["anthropic/claude-opus"]);
  await view.unmount();
});

/** A model the host reports without a provider is referenced by its bare id. */
it("writes a bare id for a model with no provider", async () => {
  const view = await mount();

  await act(async () => view.switchFor("local-llama")!.click());

  expect(view.onChangeHidden).toHaveBeenCalledWith(["local-llama"]);
  await view.unmount();
});

/** Showing a hidden model removes it from the list rather than adding it again. */
it("writes the removal when a hidden model is shown again", async () => {
  const view = await mount({ hidden: ["openai/o3", "anthropic/claude-opus"] });

  await act(async () => view.switchFor("Opus")!.click());

  expect(view.onChangeHidden).toHaveBeenCalledWith(["openai/o3"]);
  await view.unmount();
});

/**
 * The group switch covers every model in the group, not only the rows on screen:
 * a folded group's rows are not rendered, and saving just those would leave the
 * hidden ones hidden while the switch says otherwise.
 */
it("writes the whole group from its own switch", async () => {
  const view = await mount();

  // Fold the group first, so its rows are out of the DOM.
  await act(async () => view.foldFor("openai")!.click());
  expect(view.text()).not.toContain("GPT-5");

  await act(async () => view.switchFor("Toggle every model under openai")!.click());

  expect(view.onChangeHidden).toHaveBeenCalledWith(["openai/gpt-5", "openai/o3"]);
  await view.unmount();
});

/** A provider whose models are all hidden reads as hidden, and shows the count. */
it("reports how many of a group's models are hidden", async () => {
  const view = await mount({ hidden: ["openai/o3"] });

  expect(view.text()).toContain("1 hidden");
  expect(view.switchFor("Toggle every model under openai")!.getAttribute("aria-checked")).toBe("false");
  await view.unmount();
});

/** The search filters by id, label and provider, and shows folded groups. */
it("filters the list as the user types", async () => {
  const view = await mount();

  await act(async () => view.foldFor("openai")!.click());
  await view.search("opus");

  expect(view.text()).toContain("Opus");
  expect(view.text()).not.toContain("openai");

  // Clearing the search restores what was folded away.
  await view.search("");
  expect(view.text()).toContain("openai");
  await view.unmount();
});

/** An offline host cannot be asked, so its switches are closed. */
it("disables the switches while the host cannot be reached", async () => {
  const view = await mount({ online: false });

  expect(view.switchFor("Opus")!.disabled).toBe(true);
  await view.unmount();
});

/** A failed read is reported, with a way to try again. */
it("reports a failed read and offers a retry", async () => {
  listRemoteSettingsModels.mockRejectedValueOnce(new Error("peer_not_connected"));
  const view = await mount();

  expect(view.text()).toContain("peer_not_connected");

  listRemoteSettingsModels.mockResolvedValue([{ id: "gpt-5", provider: "openai" }]);
  const retry = [...view.container.querySelectorAll("button")]
    .find(node => node.textContent === "Retry")!;
  await act(async () => retry.click());

  expect(view.text()).toContain("openai");
  await view.unmount();
});

/** A host with no models says so, rather than rendering an empty page. */
it("says so when the host offers no models", async () => {
  listRemoteSettingsModels.mockResolvedValue([]);
  const view = await mount();

  expect(view.text()).toContain("No models are available");
  await view.unmount();
});

/**
 * The list this page reads is the *settings* one, and that is the fix for a real
 * bug: the host strips hidden models out of `list_models` before replying, so a
 * page fed from it could only ever show models that were already on. A model the
 * user had turned off was not in the list at all — there was no switch to turn it
 * back on, and the setting was one-way.
 */
it("reads the unfiltered settings list, not the picker's filtered one", async () => {
  const view = await mount();

  expect(listRemoteSettingsModels).toHaveBeenCalledWith("desktop_a");
  await view.unmount();
});

/**
 * And a hidden model is therefore reachable: it renders, with its switch off, so
 * turning it back on writes the removal rather than being impossible.
 */
it("shows a hidden model with its switch off, and can turn it back on", async () => {
  listRemoteSettingsModels.mockResolvedValue([
    { id: "gpt-5", label: "GPT-5", provider: "openai" },
    { id: "o3", label: "o3", provider: "openai" },
  ]);
  const view = await mount({ hidden: ["openai/o3"] });

  // Present, and off — the state the picker's list could not represent.
  expect(view.text()).toContain("o3");
  expect(view.switchFor("o3")!.getAttribute("aria-checked")).toBe("false");

  await act(async () => view.switchFor("o3")!.click());
  expect(view.onChangeHidden).toHaveBeenCalledWith([]);
  await view.unmount();
});
