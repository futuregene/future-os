// @vitest-environment jsdom
import type { RemotePeer } from "./remotePeerClient";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { beforeEach, expect, it, vi } from "vitest";
import { WorkspaceDeviceDialog } from "./WorkspaceDeviceDialog";

/**
 * The machine step in front of the workspace form.
 *
 * Reported as "the + went straight to a local folder picker": with a host
 * paired, the form behind this button browsed the wrong disk. These assertions
 * pin the two things that make the step useful — this computer is offered
 * without hiding the paired machines, and a machine that cannot take work says
 * why instead of vanishing.
 */

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
    stopped: false,
    ...overrides,
  };
}

beforeEach(() => {
  document.body.innerHTML = "";
});

async function mount(peers: RemotePeer[]) {
  const onPickLocal = vi.fn();
  const onPickPeer = vi.fn();
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  await act(async () => {
    root.render(
      <WorkspaceDeviceDialog
        onClose={() => {}}
        onPickLocal={onPickLocal}
        onPickPeer={onPickPeer}
        peers={peers}
      />,
    );
  });
  return {
    container,
    onPickLocal,
    onPickPeer,
    unmount: async () => {
      await act(async () => root.unmount());
      container.remove();
    },
  };
}

function rowFor(container: HTMLElement, label: string) {
  return container.querySelector<HTMLButtonElement>(`button[aria-label="${label}"]`);
}

it("offers this computer first, then every paired machine", async () => {
  const view = await mount([
    peer(),
    peer({ desktopId: "desktop_b", icon: "server", name: "Build box" }),
  ]);

  const dialog = view.container.querySelector("[role='dialog']")!;
  const labels = [...dialog.querySelectorAll("button[aria-label]")]
    .map(button => button.getAttribute("aria-label"));
  expect(labels).toEqual(["This computer", "Studio iMac", "Build box"]);
  await view.unmount();
});

it("names why a machine cannot take work instead of hiding it", async () => {
  const view = await mount([peer({ agentAvailable: false })]);

  // Paired, unreachable-while-restarting: the row stays, the status says which
  // kind of "not yet" it is, and it cannot be chosen.
  const row = rowFor(view.container, "Studio iMac");
  expect(row?.disabled).toBe(true);
  expect(row?.querySelector("[data-testid='option-note']")?.textContent).toBe("Agent unavailable");
  await view.unmount();
});

it("reports the machine that was chosen", async () => {
  const view = await mount([peer()]);

  await act(async () => rowFor(view.container, "This computer")!.click());
  expect(view.onPickLocal).toHaveBeenCalledTimes(1);

  await act(async () => rowFor(view.container, "Studio iMac")!.click());
  expect(view.onPickPeer).toHaveBeenCalledWith(expect.objectContaining({ desktopId: "desktop_a" }));
  await view.unmount();
});
