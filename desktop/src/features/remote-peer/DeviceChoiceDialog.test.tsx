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
 * list has to be finishable without reaching for the mouse: the dialog opens
 * with this machine drawn as chosen, the arrows walk the rows, and Enter opens
 * the chosen one. The arrow walk skips a machine that cannot take work — it is
 * drawn, with its reason, but landing on it would be choosing a row that
 * refuses the work.
 *
 * "Opens with this machine drawn as chosen" is allowed to be *only* that: the
 * row is painted, and nothing is created until a row is opened. A default that
 * acted would answer the question the dialog exists to ask.
 *
 * The chosen row is state (`data-chosen`), not the caret's `:focus-visible`:
 * WebKit does not match that pseudo-class for an element focused by script, so
 * the row this dialog opens on was drawn like every other one and the list read
 * as "nothing is selected". The last two tests hold that line — the keys work
 * with the caret on the row *and* with the caret nowhere in the list.
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
  const onClose = vi.fn();
  const onPickLocal = vi.fn();
  const onPickPeer = vi.fn();
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  await act(async () => {
    root.render(
      <DeviceChoiceDialog
        description="A conversation is stored on one computer and runs there."
        onClose={onClose}
        onPickLocal={onPickLocal}
        onPickPeer={onPickPeer}
        peers={peers}
        title="Choose a device"
      />,
    );
  });
  return {
    container,
    onClose,
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

/** The row drawn as the chosen one, by its own name — see `data-chosen`. */
function chosen() {
  const rows = [...document.querySelectorAll("[data-chosen='true']")];
  if (rows.length > 1)
    throw new Error(`more than one row is drawn as chosen: ${rows.length}`);
  return rows[0]?.getAttribute("aria-label") ?? null;
}

/**
 * Press a key where the caret is and let it bubble to the list, the way a real
 * key press arrives (`document.activeElement` rather than the dialog element:
 * what matters is that the row itself answers to it).
 */
async function press(key: string, target?: EventTarget | null) {
  await act(async () => {
    (target ?? document.activeElement ?? document.body).dispatchEvent(
      new KeyboardEvent("keydown", { bubbles: true, key }),
    );
  });
}

it("starts the caret on this machine", async () => {
  const view = await mount([peer()]);

  expect(focused()).toBe("This computer");
  await view.unmount();
});

it("draws that first row as chosen, and creates nothing until one is opened", async () => {
  const view = await mount([peer()]);

  expect(chosen()).toBe("This computer");
  expect(view.onPickLocal).not.toHaveBeenCalled();
  expect(view.onPickPeer).not.toHaveBeenCalled();
  await view.unmount();
});

it("keeps the row it opens on drawn as chosen when the caret is taken away", async () => {
  const view = await mount([peer()]);

  // The engine that does not paint `:focus-visible` for a scripted focus, and
  // the dialog that opens over a composer asking for its caret back, both come
  // down to this: the row is chosen whether or not anything is focused.
  await act(async () => (document.activeElement as HTMLElement | null)?.blur());
  expect(chosen()).toBe("This computer");
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
  expect(chosen()).toBe("Studio iMac");
  // Build box is paired and unavailable: it stays drawn, and the arrow walks
  // past it rather than landing on an answer that would be refused.
  await press("ArrowDown");
  expect(focused()).toBe("Laptop");
  expect(chosen()).toBe("Laptop");
  // The ends wrap, so neither end is a wall.
  await press("ArrowDown");
  expect(focused()).toBe("This computer");
  await press("ArrowUp");
  expect(focused()).toBe("Laptop");
  await view.unmount();
});

it("answers the arrows with no caret anywhere in the list", async () => {
  const view = await mount([peer()]);

  // The caret stayed where it was before the dialog opened (a composer behind
  // it, or nowhere at all). The list still has to be answerable: the arrows
  // move the chosen row from wherever it is drawn.
  await act(async () => (document.activeElement as HTMLElement | null)?.blur());
  await press("ArrowDown", document.body);
  expect(chosen()).toBe("Studio iMac");
  expect(focused()).toBe("Studio iMac");

  await press("Enter", document.body);
  expect(view.onPickPeer).toHaveBeenCalledWith(expect.objectContaining({ desktopId: "desktop_a" }));
  await view.unmount();
});

it("leaves the footer's own Enter to the footer", async () => {
  const view = await mount([peer()]);

  const footer = document.querySelector("[role=dialog]")?.lastElementChild;
  const cancel = footer?.querySelector("button");
  if (!cancel)
    throw new Error("the dialog has no footer button");
  await act(async () => {
    cancel.focus();
  });
  await press("Enter", cancel);
  // Cancel is answered by the button itself; picking a machine here would open
  // this computer while the reader pressed the key over "Cancel".
  expect(view.onPickLocal).not.toHaveBeenCalled();
  expect(view.onPickPeer).not.toHaveBeenCalled();
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
