// @vitest-environment jsdom
import type { RemotePeer, RemoteWorkspaceRow } from "./remotePeerClient";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { beforeEach, expect, it, vi } from "vitest";
import { RemoteWorkspaceCreateDialog } from "./RemoteWorkspaceCreateDialog";

/**
 * Creating a workspace on the machine the device step named.
 *
 * The path is typed because there is no folder picker for another computer's
 * disk; the host keeps its own rules, so its refusal is shown in the dialog
 * rather than being closed over.
 */

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

const create = vi.fn<(desktopId: string, path: string, name: string) => Promise<RemoteWorkspaceRow>>();
vi.mock("./remotePeerClient", () => ({
  createRemoteWorkspace: (...args: Parameters<typeof create>) => create(...args),
}));

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
    stopped: false,
    ...overrides,
  };
}

beforeEach(() => {
  document.body.innerHTML = "";
  create.mockReset();
});

async function mount(onCreated = vi.fn()) {
  const onClose = vi.fn();
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  await act(async () => {
    root.render(<RemoteWorkspaceCreateDialog onClose={onClose} onCreated={onCreated} peer={peer()} />);
  });
  return {
    container,
    onClose,
    onCreated,
    unmount: async () => {
      await act(async () => root.unmount());
      container.remove();
    },
  };
}

function input(container: HTMLElement, label: string) {
  return container.querySelector<HTMLInputElement>(`input[aria-label="${label}"]`)!;
}
function button(container: HTMLElement, label: string) {
  return [...container.querySelectorAll<HTMLButtonElement>("button")].find(node => node.textContent === label)!;
}
async function type(container: HTMLElement, label: string, value: string) {
  const field = input(container, label);
  await act(async () => {
    // React's onChange follows the native setter + input event in jsdom.
    const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!;
    setter.call(field, value);
    field.dispatchEvent(new Event("input", { bubbles: true }));
  });
}

it("names the machine it writes to and refuses to submit without a path", async () => {
  const view = await mount();

  expect(view.container.textContent).toContain("New workspace on Studio iMac");
  expect(button(view.container, "Create").disabled).toBe(true);

  await type(view.container, "Folder path", "/Users/you/code/project");
  expect(button(view.container, "Create").disabled).toBe(false);
  await view.unmount();
});

it("creates the typed folder on that host and reports the row", async () => {
  const created: RemoteWorkspaceRow = { id: "ws_new", name: "Project", path: "/Users/you/code/project", pinned: false };
  create.mockResolvedValue(created);
  const view = await mount();

  await type(view.container, "Folder path", "  /Users/you/code/project  ");
  await type(view.container, "Name (optional)", "Project");
  await act(async () => button(view.container, "Create").click());

  expect(create).toHaveBeenCalledWith("desktop_a", "/Users/you/code/project", "Project");
  expect(view.onCreated).toHaveBeenCalledWith(created);
  await view.unmount();
});

it("shows the host's refusal in the dialog instead of closing it", async () => {
  create.mockRejectedValue(new Error("directory does not exist"));
  const view = await mount();

  await type(view.container, "Folder path", "/Users/you/missing");
  await act(async () => button(view.container, "Create").click());

  expect(view.container.textContent).toContain("directory does not exist");
  expect(view.onCreated).not.toHaveBeenCalled();
  // Still up, with the path the user typed kept for the correction.
  expect(input(view.container, "Folder path").value).toBe("/Users/you/missing");
  await view.unmount();
});
