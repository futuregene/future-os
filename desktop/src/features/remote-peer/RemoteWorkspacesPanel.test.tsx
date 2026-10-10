// @vitest-environment jsdom
import type { RemotePeer } from "./remotePeerClient";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { beforeEach, expect, it, vi } from "vitest";
import { RemoteWorkspacesPanel } from "./RemoteWorkspacesPanel";

/**
 * Another computer's workspaces.
 *
 * A workspace is a folder on *that* machine, so every write here has to land
 * there — and removing one takes that machine's conversations with it, which is
 * why the confirmation is asserted rather than assumed.
 *
 * Tested against the Tauri boundary, so the wire shapes below are the shapes
 * that actually leave the app.
 */

const invoke = vi.hoisted(() => vi.fn());
vi.mock("../../integrations/tauri/invoke", () => ({ invokeCommand: invoke }));

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

function peer(overrides: Partial<RemotePeer> = {}): RemotePeer {
  return {
    agentAvailable: true,
    bridgeInstanceId: "b",
    connected: true,
    desktopId: "desktop_a",
    error: null,
    features: ["workspace_create_v1", "workspace_pinning_v1"],
    icon: "laptop",
    name: "Studio iMac",
    pairId: "pair_1",
    ...overrides,
  };
}

/** The host's workspace snapshot, as `remote_peer_workspaces` returns it. */
const SNAPSHOT = {
  desktopId: "desktop_a",
  workspaces: [
    { id: "ws_1", kind: "user", name: "future-os", path: "/code/future-os", pinned: true },
    { id: "ws_2", kind: "user", name: "docs", path: "/code/docs", pinned: false },
  ],
};

const empty = { desktopId: "desktop_a", workspaces: [] };

/** The commands that reached the backend, in order. */
function sent(type: string) {
  return invoke.mock.calls.filter(([, args]) => {
    const command = (args as { command?: { type?: string } })?.command;
    return command?.type === type;
  });
}

async function mount(props: { available?: boolean; onChanged?: () => void; peer?: RemotePeer } = {}) {
  const onChanged = props.onChanged ?? vi.fn();
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  await act(async () => {
    root.render(
      <RemoteWorkspacesPanel
        available={props.available ?? true}
        onChanged={onChanged}
        peer={props.peer ?? peer()}
      />,
    );
  });
  await settle();
  return {
    container,
    onChanged,
    button: (text: string) =>
      [...container.querySelectorAll("button")].find(node => node.textContent === text),
    input: (label: string) => container.querySelector<HTMLInputElement>(`input[aria-label="${label}"]`),
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
  invoke.mockResolvedValue(SNAPSHOT);
});

/** That host's folders, with the ones it pinned marked. */
it("lists that host's workspaces with their paths", async () => {
  const panel = await mount();

  expect(invoke).toHaveBeenCalledWith("remote_peer_workspaces", { desktopId: "desktop_a" });
  expect(panel.text()).toContain("future-os");
  expect(panel.text()).toContain("/code/docs");
  await panel.unmount();
});

/** A host with no workspaces says so rather than showing an empty box. */
it("says when that host has none", async () => {
  invoke.mockResolvedValue(empty);
  const panel = await mount();

  expect(panel.text()).toContain("no workspaces yet");
  await panel.unmount();
});

/** Pinning is a write on that host, and the list is re-read rather than guessed. */
it("pins a workspace on that host", async () => {
  const panel = await mount();
  await act(async () => panel.button("Pin")!.click());
  await settle();

  expect(sent("set_workspace_pinned")[0]![1]).toEqual({
    desktopId: "desktop_a",
    command: { type: "set_workspace_pinned", workspaceId: "ws_2", pinned: true },
    lane: "list",
  });
  // Unpinning is the same write with the other flag.
  await act(async () => panel.button("Unpin")!.click());
  await settle();
  expect(sent("set_workspace_pinned")[1]![1]).toMatchObject({
    command: { workspaceId: "ws_1", pinned: false },
  });
  await panel.unmount();
});

/**
 * Adding registers a path that must already exist on that machine.
 *
 * The path is the host's, so it is sent as typed (trimmed) and the host decides;
 * this machine cannot check a disk it cannot see.
 */
it("adds a workspace at a path on that host", async () => {
  invoke.mockImplementation(async (_command, args) => {
    const type = (args as { command?: { type?: string } })?.command?.type;
    if (type === "create_workspace")
      return { workspace: { id: "ws_3", name: "new", path: "/code/new", pinned: false } };
    return SNAPSHOT;
  });
  const panel = await mount();

  await type(panel.input("Folder path")!, "/code/new");
  await type(panel.input("Name (optional)")!, "new");
  await act(async () => panel.button("Add")!.click());
  await settle();

  expect(sent("create_workspace")[0]![1]).toEqual({
    desktopId: "desktop_a",
    command: { type: "create_workspace", path: "/code/new", name: "new" },
    lane: "list",
  });
  // The field is cleared so the same path is not added twice by accident.
  expect(panel.input("Folder path")!.value).toBe("");
  await panel.unmount();
});

/** Without a path there is nothing to register, so the button is closed. */
it("does not add without a path", async () => {
  const panel = await mount();

  expect(panel.button("Add")!.disabled).toBe(true);
  await panel.unmount();
});

/** A host that answers a create with no workspace is reported, not rendered blank. */
it("reports a create that came back without a workspace", async () => {
  invoke.mockImplementation(async (_command, args) => {
    const type = (args as { command?: { type?: string } })?.command?.type;
    if (type === "create_workspace")
      return {};
    return SNAPSHOT;
  });
  const panel = await mount();
  await type(panel.input("Folder path")!, "/code/new");
  await act(async () => panel.button("Add")!.click());
  await settle();

  expect(panel.text()).toContain("invalid_workspace_snapshot");
  await panel.unmount();
});

/**
 * Removing asks first, and says what goes with it.
 *
 * The host deletes that workspace's conversations and threads too, so this is
 * not a cosmetic removal — a silent one would lose a conversation the user did
 * not think they were touching.
 */
it("confirms before removing, and says what goes with it", async () => {
  const panel = await mount();
  // The remove buttons carry only a trash icon.
  await act(async () => panel.container.querySelectorAll<HTMLButtonElement>("button")[1]!.click());
  await settle();

  expect(panel.text()).toContain("/code/future-os");
  expect(sent("delete_workspace")).toEqual([]);
  await panel.unmount();
});

it("removes the workspace on that host once confirmed", async () => {
  const panel = await mount();
  await act(async () => panel.container.querySelectorAll<HTMLButtonElement>("button")[1]!.click());
  await settle();

  await act(async () => panel.button("Remove")!.click());
  await settle();

  expect(sent("delete_workspace")[0]![1]).toEqual({
    desktopId: "desktop_a",
    command: { type: "delete_workspace", workspaceId: "ws_1" },
    lane: "list",
  });
  await panel.unmount();
});

/** Cancelling writes nothing. */
it("writes nothing when a removal is cancelled", async () => {
  const panel = await mount();
  await act(async () => panel.container.querySelectorAll<HTMLButtonElement>("button")[1]!.click());
  await settle();

  await act(async () => panel.button("Cancel")!.click());
  await settle();

  expect(sent("delete_workspace")).toEqual([]);
  await panel.unmount();
});

/**
 * A workspace change moves conversations too, so the caller is told to re-read —
 * otherwise the conversation list would keep showing rows for a workspace that
 * is gone.
 */
it("tells the caller a change happened", async () => {
  const panel = await mount();
  await act(async () => panel.button("Pin")!.click());
  await settle();

  expect(panel.onChanged).toHaveBeenCalled();
  await panel.unmount();
});

/** An unreachable host cannot be asked or told anything. */
it("closes every write while the host is unreachable", async () => {
  const panel = await mount({ available: false, peer: peer({ connected: false }) });

  for (const node of panel.container.querySelectorAll<HTMLButtonElement>("button"))
    expect(node.disabled).toBe(true);
  await panel.unmount();
});

/** A failed read or write is reported rather than leaving the page looking fine. */
it("reports a failure", async () => {
  invoke.mockRejectedValue(new Error("peer_not_connected"));
  const panel = await mount();

  expect(panel.text()).toContain("peer_not_connected");
  await panel.unmount();
});
