// @vitest-environment jsdom
import type { RemotePeer } from "./remotePeerClient";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { RemotePeerSettings } from "./RemotePeerSettings";

/**
 * One computer's settings, read from that computer and written back to it.
 *
 * The rows are where a value's *owner* is easy to get wrong: everything here
 * lives on the host, so a write that does not reach it, or that reaches it with
 * the wrong shape, is the failure this file exists to catch.
 */

const request = vi.fn<(desktopId: string, command: Record<string, unknown>, lane: string) => Promise<unknown>>();
vi.mock("./remotePeerClient", async (importOriginal) => {
  const actual = await importOriginal<typeof import("./remotePeerClient")>();
  return {
    ...actual,
    requestRemotePeer: (...args: Parameters<typeof request>) => request(...args),
  };
});
// The icon picker and the skills panel are their own concerns; both are covered
// where they live.
vi.mock("./PeerIconPicker", () => ({ PeerIconPicker: () => <div data-testid="icon-picker" /> }));
vi.mock("./RemoteSkillsPanel", () => ({
  RemoteSkillsPanel: (props: { available: boolean }) => (
    <div data-available={String(props.available)} data-testid="skills-panel" />
  ),
}));

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

function peer(overrides: Partial<RemotePeer> = {}): RemotePeer {
  return {
    agentAvailable: true,
    bridgeInstanceId: "bridge_1",
    connected: true,
    desktopId: "desktop_a",
    error: null,
    features: [],
    icon: "laptop",
    name: "Studio iMac",
    pairId: "pair_1",
    ...overrides,
  };
}

/** The host's answers, by command type. */
function answerHost(overrides: Partial<Record<string, unknown>> = {}): void {
  request.mockImplementation(async (_desktopId, command) => {
    const type = typeof command.type === "string" ? command.type : "";
    if (type in overrides)
      return overrides[type];
    if (type === "get_desktop_settings")
      return { autoUpgradeSkills: true, autoTitleFirstTurn: false };
    if (type === "list_settings_models")
      return { models: ["a", "b"] };
    if (type === "list_providers")
      return { providers: ["one"] };
    if (type === "list_tasks")
      return { tasks: ["t1", "t2", "t3"] };
    return undefined;
  });
}

async function mount(props: {
  onChanged?: () => void;
  onUnpair?: () => void;
  onUpdateLabel?: (patch: { name?: string; icon?: string }) => void;
  peer?: RemotePeer;
} = {}) {
  const onChanged = props.onChanged ?? vi.fn();
  const onUnpair = props.onUnpair ?? vi.fn();
  const onUpdateLabel = props.onUpdateLabel ?? vi.fn();
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  await act(async () => {
    root.render(
      <RemotePeerSettings
        onBack={vi.fn()}
        onChanged={onChanged}
        onUnpair={onUnpair}
        onUpdateLabel={onUpdateLabel}
        peer={props.peer ?? peer()}
      />,
    );
  });
  await settle();
  return {
    container,
    onChanged,
    onUnpair,
    onUpdateLabel,
    input: () => container.querySelector<HTMLInputElement>("input")!,
    switchFor: (label: string) => container.querySelector<HTMLButtonElement>(`button[aria-label="${label}"]`),
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
  request.mockReset();
  answerHost();
});

/** The counts are the host's answer, not a guess from what this machine has. */
it("reads the host's own settings and counts", async () => {
  const view = await mount();

  expect(request).toHaveBeenCalledWith("desktop_a", { type: "get_desktop_settings" }, "list");
  expect(request).toHaveBeenCalledWith("desktop_a", { type: "list_settings_models" }, "list");
  expect(request).toHaveBeenCalledWith("desktop_a", { type: "list_tasks" }, "list");

  // Two models, one provider, three tasks — as the host reported them.
  expect(view.text()).toContain("2");
  expect(view.text()).toContain("3");
  await view.unmount();
});

describe("the host's automation settings", () => {
  it("shows what the host has, and writes a toggle back to it", async () => {
    const view = await mount();
    const toggle = view.switchFor("Auto-upgrade skills")!;
    expect(toggle.getAttribute("aria-checked")).toBe("true");

    await act(async () => toggle.click());
    await settle();

    expect(request).toHaveBeenCalledWith(
      "desktop_a",
      expect.objectContaining({
        type: "update_desktop_settings",
        settings: expect.objectContaining({ autoUpgradeSkills: false }),
      }),
      "list",
    );
    // The caller is told so it can re-read whatever it shows about this host.
    expect(view.onChanged).toHaveBeenCalled();
    await view.unmount();
  });

  it("writes only the key it changed", async () => {
    const view = await mount();
    await act(async () => view.switchFor("Auto-title first turn")!.click());
    await settle();

    const calls = request.mock.calls;
    const settings = (calls[calls.length - 1]![1] as { settings: Record<string, unknown> }).settings;
    // A whole-object write would silently revert whatever the host changed while
    // this page was open.
    expect(Object.keys(settings)).toEqual(["autoTitleFirstTurn"]);
    await view.unmount();
  });

  it("reports a refused write", async () => {
    answerHost({ update_desktop_settings: undefined });
    request.mockImplementation(async (_desktopId, command) => {
      const type = typeof command.type === "string" ? command.type : "";
      if (type === "update_desktop_settings")
        throw new Error("desktop_settings_read_only");
      if (type === "get_desktop_settings")
        return { autoUpgradeSkills: true };
      return undefined;
    });
    const view = await mount();

    await act(async () => view.switchFor("Auto-upgrade skills")!.click());
    await settle();

    expect(view.text()).toContain("desktop_settings_read_only");
    await view.unmount();
  });
});

it("renames the machine locally, without sending the name anywhere", async () => {
  const view = await mount();
  const input = view.input();

  await act(async () => {
    const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!;
    setter.call(input, "Workstation");
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
  await act(async () => {
    const save = [...view.container.querySelectorAll("button")]
      .find(node => node.textContent === "Save")!;
    save.click();
  });

  expect(view.onUpdateLabel).toHaveBeenCalledWith(expect.objectContaining({ name: "Workstation" }));
  await view.unmount();
});

/** The skills panel is handed to the page, and told whether the host is reachable. */
it("includes the skills panel, gated on the host being connected", async () => {
  const connected = await mount();
  expect(connected.container.querySelector("[data-testid='skills-panel']")!.getAttribute("data-available")).toBe("true");
  await connected.unmount();

  const offline = await mount({ peer: peer({ connected: false }) });
  expect(offline.container.querySelector("[data-testid='skills-panel']")!.getAttribute("data-available")).toBe("false");
  await offline.unmount();
});

/** A page-level read failure is reported once, rather than as a row that looks empty. */
it("reports a failed read of the host's settings", async () => {
  request.mockRejectedValue(new Error("peer_not_connected"));
  const view = await mount();

  expect(view.text()).toContain("peer_not_connected");
  await view.unmount();
});

it("unpairs through the caller", async () => {
  const view = await mount();
  const unpair = [...view.container.querySelectorAll("button")]
    .find(node => node.textContent === "Unpair")!;

  await act(async () => unpair.click());

  expect(view.onUnpair).toHaveBeenCalled();
  await view.unmount();
});
