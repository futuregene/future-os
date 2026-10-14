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
// The *real* helpers are kept (the mock above only replaces the one the page
// calls directly) so that the parsing is exercised rather than re-implemented
// here. Their calls land on the Tauri boundary, which is mocked instead.
const invoke = vi.hoisted(() => vi.fn());
vi.mock("../../integrations/tauri/invoke", () => ({ invokeCommand: invoke }));
// The icon picker and the skills panel are their own concerns; both are covered
// where they live.
vi.mock("./PeerIconPicker", () => ({ PeerIconPicker: () => <div data-testid="icon-picker" /> }));
vi.mock("./RemoteModelsPanel", () => ({
  RemoteModelsPanel: (props: { hidden: string[]; onChangeHidden: (next: string[]) => void }) => (
    <div data-hidden={props.hidden.join(",")} data-testid="models-panel">
      <button onClick={() => props.onChangeHidden(["openai/gpt-5"])} type="button">hide-model</button>
    </div>
  ),
}));
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
    if (type === "get_settings")
      return { approvalTier: "manual", sandboxAvailable: true };
    if (type === "set_approval_tier")
      return { approvalTier: "off" };
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
  onOpenTasks?: () => void;
  onUnpair?: () => void;
  onUpdateLabel?: (patch: { name?: string; icon?: string }) => void;
  peer?: RemotePeer;
} = {}) {
  const onChanged = props.onChanged ?? vi.fn();
  const onOpenTasks = props.onOpenTasks ?? vi.fn();
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
        onOpenTasks={onOpenTasks}
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
    onOpenTasks,
    onUnpair,
    onUpdateLabel,
    input: () => container.querySelector<HTMLInputElement>("input")!,
    switchFor: (label: string) => container.querySelector<HTMLButtonElement>(`button[aria-label="${label}"]`),
    // A row on the settings page that opens a page of its own.
    link: (title: string) => [...container.querySelectorAll("button")]
      .find(node => node.textContent?.startsWith(title)),
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
  invoke.mockReset();
  invoke.mockResolvedValue(undefined);
  answerHost();
});

/** The counts are the host's answer, not a guess from what this machine has. */
it("reads the host's own settings, and only those", async () => {
  const view = await mount();

  expect(request).toHaveBeenCalledWith("desktop_a", { type: "get_desktop_settings" }, "list");
  // The long lists are read by their own pages, not here: reading them on the
  // settings page would make it wait for data it does not show.
  expect(request).not.toHaveBeenCalledWith("desktop_a", { type: "list_settings_models" }, "list");
  expect(request).not.toHaveBeenCalledWith("desktop_a", { type: "list_models" }, "list");
  expect(request).not.toHaveBeenCalledWith("desktop_a", { type: "list_tasks" }, "list");
  expect(request).not.toHaveBeenCalledWith("desktop_a", { type: "list_providers" }, "list");
  await view.unmount();
});

/**
 * Tasks are managed in place rather than counted.
 *
 * The count was a dead end: it named a number the user could do nothing about,
 * on a page that already knows how to reach the machine.
 */
it("opens that host's tasks from the host-content section", async () => {
  const view = await mount({ peer: peer({ connected: true }) });
  await act(async () => view.link("Tasks")!.click());

  expect(view.onOpenTasks).toHaveBeenCalled();
  await view.unmount();
});

/** An unreachable host cannot be asked for its tasks, so the entry is closed. */
it("refuses to open tasks while the host is offline", async () => {
  const view = await mount({ peer: peer({ connected: false }) });
  expect(view.link("Tasks")!.disabled).toBe(true);
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

/**
 * The icon row puts its control below the label rather than beside it.
 *
 * The picker is two dozen glyphs — wider than this column — and beside a label it
 * took every pixel of it: the label was drawn one character per line and the grid
 * ran out of the card. jsdom lays nothing out, so what is asserted here is the
 * shape (this row stacks, the rows around it do not); the widths are measured in
 * the screenshot harness, on the same row.
 */
it("stacks the icon picker under its label instead of beside it", async () => {
  const view = await mount();

  const picker = view.container.querySelector("[data-testid='icon-picker']")!;
  const row = picker.parentElement!.parentElement!;
  expect(row.className).toContain("flex-col");
  // Left-aligned with the label it sits under, not pushed to the row's right edge.
  expect(picker.parentElement!.className).toContain("justify-start");
  // And its neighbours keep the row shape: a switch stays on the right of its label.
  expect(row.previousElementSibling!.className).toContain("items-center");
  expect(row.previousElementSibling!.className).not.toContain("flex-col");
  await view.unmount();
});

/** The skills panel is handed to the page, and told whether the host is reachable. */
it("reaches the skills page, and tells it whether the host is connected", async () => {
  const connected = await mount();
  await act(async () => connected.link("Skills")!.click());
  expect(connected.container.querySelector("[data-testid='skills-panel']")!.getAttribute("data-available")).toBe("true");
  await connected.unmount();

  const offline = await mount({ peer: peer({ connected: false }) });
  // The row is still offered, and disabled: a user who paired an older, or a
  // disconnected, build should see the capability exists.
  expect(offline.link("Skills")!.disabled).toBe(true);
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

/**
 * The other computer's approval mode.
 *
 * It reads as its own section because the value belongs to that host: this
 * machine has an approval mode too, and the two are different answers to the
 * same question. What is asserted is that the page shows the *host's* mode, that
 * it writes through, and that the host's answer wins over the request.
 *
 * These run through the real client helpers rather than a stubbed one, so the
 * wire shape below is the shape that actually leaves the app.
 */
describe("the host's approval mode", () => {
  const withApproval = (overrides: Partial<RemotePeer> = {}) =>
    peer({ features: ["approval_tier_v1", "auto_approval_v1"], ...overrides });

  const select = (container: HTMLElement) => container.querySelector<HTMLSelectElement>("select")!;

  /** The command that reached the backend, whichever helper sent it. */
  const sent = (type: string) =>
    invoke.mock.calls
      .map(([, args]) => args as { command?: Record<string, unknown> })
      .filter(args => args?.command?.type === type);

  function answerHostApproval(get: unknown, set?: unknown): void {
    invoke.mockImplementation(async (_cmd, args) => {
      const type = (args as { command?: { type?: string } })?.command?.type;
      if (type === "get_settings")
        return get;
      if (type === "set_approval_tier")
        return set;
      return undefined;
    });
  }

  /** A host that does not advertise the command must not be offered the rows. */
  it("is offered only by a host that serves it", async () => {
    const view = await mount({ peer: peer({ features: [] }) });
    expect(select(view.container)).toBeNull();
    expect(sent("get_settings")).toEqual([]);
    await view.unmount();
  });

  it("reads the mode that host is in", async () => {
    answerHostApproval({ approvalTier: "off", sandboxAvailable: true });
    const view = await mount({ peer: withApproval() });

    expect(select(view.container).value).toBe("off");
    expect(sent("get_settings")[0]).toEqual({
      desktopId: "desktop_a",
      command: { type: "get_settings" },
      lane: "list",
    });
    await view.unmount();
  });

  it("writes a change back to that host, and shows what it answered", async () => {
    answerHostApproval({ approvalTier: "manual", sandboxAvailable: true }, { approvalTier: "auto" });
    const view = await mount({ peer: withApproval() });
    expect(select(view.container).value).toBe("manual");

    await act(async () => {
      select(view.container).value = "auto";
      select(view.container).dispatchEvent(new Event("change", { bubbles: true }));
    });
    await settle();

    expect(sent("set_approval_tier")[0]).toEqual({
      desktopId: "desktop_a",
      command: { type: "set_approval_tier", tier: "auto" },
      lane: "list",
    });
    // `auto` is what the host settled on, and so what is shown.
    expect(select(view.container).value).toBe("auto");
    await view.unmount();
  });

  /**
   * The host may refuse to run in the mode it was asked for — asking for
   * `sandbox` on a host whose sandbox turned out to be missing is answered with
   * `manual`. Showing the request would report a mode the host is not in.
   */
  it("shows the mode the host settled on, not the one asked for", async () => {
    answerHostApproval({ approvalTier: "manual", sandboxAvailable: true }, { approvalTier: "manual" });
    const view = await mount({ peer: withApproval() });

    await act(async () => {
      select(view.container).value = "sandbox";
      select(view.container).dispatchEvent(new Event("change", { bubbles: true }));
    });
    await settle();

    expect(select(view.container).value).toBe("manual");
    await view.unmount();
  });

  /** A host without a sandbox cannot run sandboxed, so those modes are not offered. */
  it("disables the modes the host cannot run in", async () => {
    answerHostApproval({ approvalTier: "manual", sandboxAvailable: false });
    const view = await mount({ peer: withApproval() });

    const options = [...select(view.container).options];
    const byValue = (value: string) => options.find(option => option.value === value)!;
    expect(byValue("manual").disabled).toBe(false);
    expect(byValue("off").disabled).toBe(false);
    expect(byValue("sandbox").disabled).toBe(true);
    expect(byValue("auto").disabled).toBe(true);
    await view.unmount();
  });

  /** Automatic review needs the host's own capability, not just a sandbox. */
  it("disables automatic review on a host that does not have it", async () => {
    answerHostApproval({ approvalTier: "manual", sandboxAvailable: true });
    const view = await mount({ peer: peer({ features: ["approval_tier_v1"] }) });

    const options = [...select(view.container).options];
    expect(options.find(option => option.value === "auto")!.disabled).toBe(true);
    expect(options.find(option => option.value === "sandbox")!.disabled).toBe(false);
    await view.unmount();
  });

  /** A refused write is reported, and the shown mode stays whatever the host said. */
  it("reports a refused write without changing what the host reported", async () => {
    invoke.mockImplementation(async (_cmd, args) => {
      const type = (args as { command?: { type?: string } })?.command?.type;
      if (type === "get_settings")
        return { approvalTier: "manual", sandboxAvailable: true };
      if (type === "set_approval_tier")
        throw new Error("invalid tier");
      return undefined;
    });
    const view = await mount({ peer: withApproval() });

    await act(async () => {
      select(view.container).value = "off";
      select(view.container).dispatchEvent(new Event("change", { bubbles: true }));
    });
    await settle();

    expect(view.text()).toContain("invalid tier");
    expect(select(view.container).value).toBe("manual");
    await view.unmount();
  });
});

/**
 * The long sections are pages of their own.
 *
 * As sections in this column, a machine's skills, models, providers and
 * workspaces sat between the settings above them and the settings below, and the
 * user scrolled past a list they did not come for to reach the next switch.
 */
describe("the long sections are their own pages", () => {
  it("offers a row for each, and no longer renders them inline", async () => {
    const view = await mount({
      peer: peer({ features: ["provider_management_v1", "workspace_create_v1"] }),
    });

    // The rows name what is behind them.
    for (const title of ["Models", "Skills", "Providers and keys", "Workspaces", "Tasks"])
      expect(view.link(title), title).toBeTruthy();

    // And the page itself stays short: the skills panel is not here until asked
    // for, and the models read is not made on its behalf.
    expect(view.container.querySelector("[data-testid='skills-panel']")).toBeNull();
    expect(request).not.toHaveBeenCalledWith("desktop_a", { type: "list_models" }, "list");
    await view.unmount();
  });

  it("opens a section, and names the page it opened", async () => {
    const view = await mount();

    await act(async () => view.link("Skills")!.click());

    // The heading says which page this is, so a list reached by a click is not a
    // list with no name; the back control says where it goes.
    expect(view.text()).toContain("Settings");
    expect(view.container.querySelector("[data-testid='skills-panel']")).not.toBeNull();
    // The settings the user just left are gone, so scrolling cannot mix the two.
    expect(view.switchFor("Auto-upgrade skills")).toBeNull();
    await view.unmount();
  });

  it("goes back to the settings it came from", async () => {
    const view = await mount();

    await act(async () => view.link("Skills")!.click());
    const back = [...view.container.querySelectorAll("button")]
      .find(node => node.textContent?.includes("Settings"))!;
    await act(async () => back.click());

    expect(view.container.querySelector("[data-testid='skills-panel']")).toBeNull();
    expect(view.switchFor("Auto-upgrade skills")).toBeTruthy();
    await view.unmount();
  });

  /** A host that cannot serve a section does not offer a page for it. */
  it("offers only the sections the host advertises", async () => {
    const view = await mount({ peer: peer({ features: ["workspace_create_v1"] }) });

    expect(view.link("Workspaces")).toBeTruthy();
    // Providers need the feature flag; without it the command would be refused.
    // Absent, not disabled: there is no page to open at all.
    expect(view.link("Providers and keys")).toBeUndefined();
    await view.unmount();
  });
});

/**
 * The models page reads the host's visibility list from the settings this page
 * already fetched, rather than making a second read of its own — and what it
 * writes goes back through the same `update`, so there is one write path.
 *
 * A settings payload whose `hiddenModels` is not a list of strings (an older host,
 * a hand-edited store) must not reach the page as one: it would render an entry
 * per character.
 */
describe("the models page's wiring", () => {
  it("opens it, hands it the host's hidden list, and writes back through update", async () => {
    answerHost({ get_desktop_settings: { hiddenModels: ["openai/gpt-5", "anthropic/claude-opus"] } });
    const view = await mount({ peer: peer({ features: [] }) });

    await act(async () => view.link("Models")!.click());
    expect(view.container.querySelector("[data-testid='models-panel']")!.getAttribute("data-hidden"))
      .toBe("openai/gpt-5,anthropic/claude-opus");

    await act(async () => {
      [...view.container.querySelectorAll("button")]
        .find(node => node.textContent === "hide-model")!
        .click();
    });

    expect(request).toHaveBeenCalledWith(
      "desktop_a",
      { type: "update_desktop_settings", settings: { hiddenModels: ["openai/gpt-5"] } },
      "list",
    );
    await view.unmount();
  });

  it("treats a hidden list that is not a list of strings as empty", async () => {
    answerHost({ get_desktop_settings: { hiddenModels: "openai/gpt-5" } });
    const view = await mount();

    await act(async () => view.link("Models")!.click());

    // A string would otherwise be spread into one entry per character.
    expect(view.container.querySelector("[data-testid='models-panel']")!.getAttribute("data-hidden")).toBe("");
    await view.unmount();
  });
});

/** Providers and workspaces are pages too, opened by their own rows. */
it("opens the providers page", async () => {
  const view = await mount({ peer: peer({ features: ["provider_management_v1"] }) });

  await act(async () => view.link("Providers and keys")!.click());

  expect(view.text()).toContain("Providers and keys");
  // The settings it came from are not rendered behind it.
  expect(view.switchFor("Auto-upgrade skills")).toBeNull();
  await view.unmount();
});

it("opens the workspaces page", async () => {
  const view = await mount({ peer: peer({ features: ["workspace_create_v1"] }) });

  await act(async () => view.link("Workspaces")!.click());

  expect(view.text()).toContain("Workspaces");
  expect(view.switchFor("Auto-upgrade skills")).toBeNull();
  await view.unmount();
});
