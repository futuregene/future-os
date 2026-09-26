// @vitest-environment jsdom
import type { ComponentProps } from "react";
import type { FloatingScrollbarState } from "../../lib/useFloatingScrollbar";
import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
import { describe, expect, it, vi } from "vitest";
import { FloatingScrollbar } from "./FloatingScrollbar";

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

type ScrollbarProps = ComponentProps<typeof FloatingScrollbar>;

function mount(props: ScrollbarProps) {
  const container = document.createElement("div");
  document.body.appendChild(container);
  const root = createRoot(container);
  act(() => root.render(createElement(FloatingScrollbar, props)));
  const track = () => container.querySelector<HTMLElement>("div")!;
  const thumb = () => track().querySelector<HTMLElement>("div")!;
  return {
    container,
    rerender: (next: FloatingScrollbarState) =>
      act(() => root.render(createElement(FloatingScrollbar, { onPointerDown: props.onPointerDown, scrollbar: next }))),
    thumb,
    track,
    unmount: () => {
      act(() => root.unmount());
      container.remove();
    },
  };
}

function state(overrides: Partial<FloatingScrollbarState> = {}): FloatingScrollbarState {
  return { height: 0, top: 0, visible: false, ...overrides };
}

describe("floating scrollbar", () => {
  // The two conditional class expressions are the whole behaviour of this
  // component, so all four combinations have to be asserted explicitly: a
  // 100%-line / 50%-branch reading means only half of them ever ran.
  it.each([
    { height: 0, label: "no overflow", pointerEvents: "pointer-events-none", visible: false, visibleClass: "opacity-0" },
    { height: 120, label: "overflowing", pointerEvents: "pointer-events-auto", visible: true, visibleClass: "opacity-80" },
    { height: 120, label: "overflowing but hidden", pointerEvents: "pointer-events-auto", visible: false, visibleClass: "opacity-0" },
    { height: 0, label: "no overflow though flagged visible", pointerEvents: "pointer-events-none", visible: true, visibleClass: "opacity-80" },
  ])("$label: $pointerEvents track with a $visibleClass thumb", ({ height, pointerEvents, visible, visibleClass }) => {
    const view = mount({ onPointerDown: vi.fn(), scrollbar: state({ height, top: 8, visible }) });
    expect(view.track().className).toContain(pointerEvents);
    expect(view.thumb().className).toContain(visibleClass);
    view.unmount();
  });

  it("keeps the grabbable cursor only while there is a thumb to grab", () => {
    const idle = mount({ onPointerDown: vi.fn(), scrollbar: state({ height: 0 }) });
    expect(idle.track().className).not.toContain("cursor-grab");
    idle.unmount();

    const draggable = mount({ onPointerDown: vi.fn(), scrollbar: state({ height: 36 }) });
    expect(draggable.track().className).toContain("cursor-grab");
    expect(draggable.track().className).toContain("active:cursor-grabbing");
    draggable.unmount();
  });

  it("positions the track from the measured geometry", () => {
    const view = mount({ onPointerDown: vi.fn(), scrollbar: state({ height: 42, top: 130 }) });
    expect(view.track().style.height).toBe("42px");
    expect(view.track().style.transform).toBe("translateY(130px)");
    view.unmount();
  });

  it("repositions when the measured geometry changes", () => {
    const view = mount({ onPointerDown: vi.fn(), scrollbar: state({ height: 40, top: 0 }) });
    expect(view.track().style.transform).toBe("translateY(0px)");

    view.rerender(state({ height: 64, top: 12, visible: true }));
    expect(view.track().style.height).toBe("64px");
    expect(view.track().style.transform).toBe("translateY(12px)");
    expect(view.thumb().className).toContain("opacity-80");
    view.unmount();
  });

  it("forwards a thumb drag to the caller", () => {
    const onPointerDown = vi.fn();
    const view = mount({ onPointerDown, scrollbar: state({ height: 40 }) });
    act(() => {
      view.track().dispatchEvent(new PointerEvent("pointerdown", { bubbles: true }));
    });
    expect(onPointerDown).toHaveBeenCalledTimes(1);
    // The event handed to the handler is the real React pointer event, so the
    // drag maths in `useFloatingScrollbar` can read clientY/pointerId off it.
    expect(onPointerDown.mock.calls[0]![0]).toMatchObject({ type: "pointerdown" });
    view.unmount();
  });

  it("stays inert without a thumb, holding the position it was given", () => {
    const onPointerDown = vi.fn();
    const view = mount({ onPointerDown, scrollbar: state({ height: 0, top: 0 }) });
    act(() => {
      view.track().dispatchEvent(new PointerEvent("pointerdown", { bubbles: true }));
    });
    // `pointer-events-none` is what makes this unreachable for a real pointer;
    // the handler is still wired, so the assertion is on the class that stops
    // delivery rather than on a call count.
    expect(view.track().className).toContain("pointer-events-none");
    expect(view.track().style.height).toBe("0px");
    view.unmount();
  });
});
