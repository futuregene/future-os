import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
// @vitest-environment jsdom
import { describe, expect, it, vi } from "vitest";
import { useDismissableLayer } from "./useDismissableLayer";

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

function mount(enabled: boolean, onDismiss: () => void) {
  const container = document.createElement("div");
  document.body.appendChild(container);
  const root = createRoot(container);
  let layerEl!: HTMLDivElement;
  function Probe() {
    const ref = useDismissableLayer<HTMLDivElement>({ enabled, onDismiss });
    return createElement("div", {
      ref: (el: HTMLDivElement | null) => {
        layerEl = el!;
        return ref.current = el;
      },
    });
  }
  act(() => {
    root.render(createElement(Probe));
  });
  return { container, root, layerEl };
}

describe("useDismissableLayer", () => {
  it("dismisses on an outside pointerdown", () => {
    const onDismiss = vi.fn();
    const { root } = mount(true, onDismiss);
    act(() => {
      document.body.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true }));
    });
    expect(onDismiss).toHaveBeenCalledTimes(1);
    act(() => root.unmount());
  });

  it("does not dismiss on an inside pointerdown", () => {
    const onDismiss = vi.fn();
    const { layerEl, root } = mount(true, onDismiss);
    act(() => {
      layerEl.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true }));
    });
    expect(onDismiss).not.toHaveBeenCalled();
    act(() => root.unmount());
  });

  it("ignores non-Node pointerdown targets", () => {
    const onDismiss = vi.fn();
    const { root } = mount(true, onDismiss);
    // jsdom's Document IS a Node, so forge a non-Node target by shadowing the
    // event's own `target` (dispatchEvent only sets the internal slot).
    const event = new PointerEvent("pointerdown", { bubbles: true });
    Object.defineProperty(event, "target", { value: window });
    act(() => {
      document.body.dispatchEvent(event);
    });
    expect(onDismiss).not.toHaveBeenCalled();
    act(() => root.unmount());
  });

  it("dismisses on Escape and stops propagation", () => {
    const onDismiss = vi.fn();
    const { root } = mount(true, onDismiss);
    const windowListener = vi.fn();
    window.addEventListener("keydown", windowListener);
    act(() => {
      document.body.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    });
    expect(onDismiss).toHaveBeenCalledTimes(1);
    expect(windowListener).not.toHaveBeenCalled();
    window.removeEventListener("keydown", windowListener);
    act(() => root.unmount());
  });

  it("ignores non-Escape keys", () => {
    const onDismiss = vi.fn();
    const { root } = mount(true, onDismiss);
    act(() => {
      document.body.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
    });
    expect(onDismiss).not.toHaveBeenCalled();
    act(() => root.unmount());
  });

  it("installs no listeners when disabled", () => {
    const onDismiss = vi.fn();
    const { root } = mount(false, onDismiss);
    act(() => {
      document.body.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true }));
      document.body.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    });
    expect(onDismiss).not.toHaveBeenCalled();
    act(() => root.unmount());
  });

  it("removes listeners on unmount", () => {
    const onDismiss = vi.fn();
    const { root } = mount(true, onDismiss);
    act(() => root.unmount());
    document.body.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true }));
    expect(onDismiss).not.toHaveBeenCalled();
  });
});
