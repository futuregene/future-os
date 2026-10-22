// @vitest-environment jsdom
import type { RemotePeer } from "./remotePeerClient";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { beforeEach, expect, it, vi } from "vitest";
import { DeviceChoiceDialog } from "./DeviceChoiceDialog";

/**
 * The machine step, answered from the keyboard.
 *
 * ⌘N opens this dialog, so the user's hands are already on the keyboard and the
 * list has to be finishable without reaching for the mouse: the caret starts on
 * this machine, the arrows walk the rows, and Enter opens the one the caret is
 * on. The arrow walk skips a machine that cannot take work — it is drawn, with
 * its reason, but landing on it would be focusing a row that refuses the work.
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
      <DeviceChoiceDialog
        description="A conversation is stored on one computer and runs there."
        onClose={() => {}}
        onPickLocal={onPickLocal}
        onPickPeer={onPickPeer}
        peers={peers}
        title="Choose a device"
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

/** The row the caret is on, by its own name. */
function focused() {
  return document.activeElement?.getAttribute("aria-label");
}

/**
 * Press a key where the caret is and let it bubble to the list, the way a real
 * key press arrives (`document.activeElement` rather than the dialog element:
 * what matters is that the row itself answers to it).
 */
async function press(key: string) {
  await act(async () => {
    (document.activeElement ?? document.body).dispatchEvent(
      new KeyboardEvent("keydown", { bubbles: true, key }),
    );
  });
}

it("starts the caret on this machine", async () => {
  const view = await mount([peer()]);

  expect(focused()).toBe("This computer");
  await view.unmount();
});

it("walks the rows with the arrows and skips a machine that cannot take work", async () => {
  const view = await mount([
    peer(),
    peer({ agentAvailable: false, desktopId: "desktop_b", name: "Build box" }),
    peer({ desktopId: "desktop_c", name: "Laptop" }),
  ]);

  await press("ArrowDown");
  expect(focused()).toBe("Studio iMac");
  // Build box is paired and unavailable: it stays drawn, and the arrow walks
  // past it rather than landing on an answer that would be refused.
  await press("ArrowDown");
  expect(focused()).toBe("Laptop");
  // The ends wrap, so neither end is a wall.
  await press("ArrowDown");
  expect(focused()).toBe("This computer");
  await press("ArrowUp");
  expect(focused()).toBe("Laptop");
  await view.unmount();
});

it("opens the machine Enter lands on", async () => {
  const view = await mount([peer()]);

  await press("ArrowDown");
  await press("Enter");
  expect(view.onPickPeer).toHaveBeenCalledWith(expect.objectContaining({ desktopId: "desktop_a" }));
  expect(view.onPickLocal).not.toHaveBeenCalled();
  await view.unmount();
});

it("opens this machine with Enter, the row it starts on", async () => {
  const view = await mount([peer()]);

  await press("Enter");
  expect(view.onPickLocal).toHaveBeenCalledTimes(1);
  expect(view.onPickPeer).not.toHaveBeenCalled();
  await view.unmount();
});
