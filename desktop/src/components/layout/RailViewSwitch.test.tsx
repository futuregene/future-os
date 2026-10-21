// @vitest-environment jsdom
import type { ComponentProps } from "react";
import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import { RailViewSwitch } from "./RailViewSwitch";

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

type SwitchProps = ComponentProps<typeof RailViewSwitch>;

/**
 * jsdom does no layout, so every segment reports a zero-sized rect at 0,0 and a
 * pointer over either of them is a pointer over neither. Lay the two segments
 * out by hand — side by side, in the order they were rendered — so the drag can
 * be aimed at a real half of the switch.
 */
function layOut(container: HTMLElement) {
  // `[data-testid^='rail-view-']` would also match the switch's own group, so
  // take the buttons inside it and nothing else.
  const segments = [...container.querySelectorAll<HTMLButtonElement>("[data-testid='rail-view-switch'] button")];
  segments.forEach((node, index) => {
    const left = index * 100;
    node.getBoundingClientRect = () => ({
      bottom: 24,
      height: 24,
      left,
      right: left + 100,
      top: 0,
      width: 100,
      x: left,
      y: 0,
      toJSON: () => ({}),
    }) as DOMRect;
  });
}

function mount(props: Partial<SwitchProps> = {}) {
  const container = document.createElement("div");
  document.body.appendChild(container);
  const root = createRoot(container);
  const all: SwitchProps = {
    onChange: vi.fn(),
    onMove: vi.fn(),
    order: ["workspace", "recent"],
    view: "workspace",
    ...props,
  };
  act(() => root.render(createElement(RailViewSwitch, all)));
  layOut(container);
  const segment = (id: string) => container.querySelector<HTMLButtonElement>(`[data-testid='rail-view-${id}']`)!;
  return {
    all,
    container,
    segment,
    unmount: () => {
      act(() => root.unmount());
      container.remove();
    },
  };
}

function pointer(node: HTMLElement, type: "pointerdown" | "pointermove" | "pointerup", x: number, y = 12) {
  act(() => {
    node.dispatchEvent(new PointerEvent(type, { bubbles: true, button: 0, clientX: x, clientY: y, pointerId: 1 }));
  });
}

afterEach(() => {
  document.body.innerHTML = "";
});

describe("the rail's view switch", () => {
  it("draws the segments in the order it was given", () => {
    const view = mount({ order: ["recent", "workspace"] });
    const drawn = [...view.container.querySelectorAll("[data-testid='rail-view-switch'] button")]
      .map(node => node.getAttribute("data-testid"));
    expect(drawn).toEqual(["rail-view-recent", "rail-view-workspace"]);
    view.unmount();
  });

  it("switches the list on a plain press", () => {
    const view = mount({ view: "workspace" });
    const workspace = view.segment("workspace");
    pointer(workspace, "pointerdown", 50);
    pointer(workspace, "pointerup", 50);
    act(() => workspace.click());

    expect(view.all.onChange).toHaveBeenCalledWith("workspace");
    expect(view.all.onMove).not.toHaveBeenCalled();
    view.unmount();
  });

  it("exchanges the two segments when one is dragged onto the other", () => {
    const view = mount();
    const workspace = view.segment("workspace");
    pointer(workspace, "pointerdown", 50);
    // Past the threshold, then into the other segment's half of the switch.
    pointer(workspace, "pointermove", 56);
    pointer(workspace, "pointermove", 150);

    expect(view.all.onMove).toHaveBeenCalledWith("workspace", "recent");
    view.unmount();
  });

  it("keeps the drag alive when the pointer drifts off the strip", () => {
    // A 24px-tall strip is easy to leave while dragging sideways; the exchange
    // follows which half the pointer is in, not whether it stayed in the band.
    const view = mount();
    const workspace = view.segment("workspace");
    pointer(workspace, "pointerdown", 50);
    pointer(workspace, "pointermove", 56);
    pointer(workspace, "pointermove", 150, 400);

    expect(view.all.onMove).toHaveBeenCalledWith("workspace", "recent");
    view.unmount();
  });

  it("does not switch the list with the click that ends the drag", () => {
    // A drag finishes with a pointerup and a click on the segment it started
    // from; honouring that click would change the list as well as the order.
    const view = mount();
    const workspace = view.segment("workspace");
    pointer(workspace, "pointerdown", 50);
    pointer(workspace, "pointermove", 56);
    pointer(workspace, "pointermove", 150);
    pointer(workspace, "pointerup", 150);
    act(() => workspace.click());

    expect(view.all.onChange).not.toHaveBeenCalled();
    view.unmount();
  });

  it("treats a wobble inside the segment as a press, not a drag", () => {
    // 3px of travel is a click with an unsteady hand; a reorder here would make
    // switching lists feel unreliable.
    const view = mount();
    const workspace = view.segment("workspace");
    pointer(workspace, "pointerdown", 50);
    pointer(workspace, "pointermove", 53);
    pointer(workspace, "pointermove", 52);

    expect(view.all.onMove).not.toHaveBeenCalled();
    view.unmount();
  });

  it("swaps with Alt+Arrow for a reader without a pointer", () => {
    const view = mount();
    act(() => {
      view.segment("recent").dispatchEvent(new KeyboardEvent("keydown", { altKey: true, bubbles: true, key: "ArrowLeft" }));
    });
    expect(view.all.onMove).toHaveBeenCalledWith("recent", "workspace");
    view.unmount();
  });

  it("ignores an arrow that has no segment beyond it", () => {
    const view = mount();
    const workspace = view.segment("workspace");
    act(() => {
      workspace.dispatchEvent(new KeyboardEvent("keydown", { altKey: true, bubbles: true, key: "ArrowLeft" }));
      workspace.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, key: "ArrowRight" }));
    });
    expect(view.all.onMove).not.toHaveBeenCalled();
    view.unmount();
  });
});
