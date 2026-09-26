// @vitest-environment jsdom
/**
 * Direct behavioural tests for the overlay layer stack.
 *
 * The stack exists so that only the *topmost* Overlay answers Escape: window
 * level keydown listeners cannot rely on DOM nesting, so an inner dialog's
 * Escape would otherwise also dismiss its parent. These tests drive the public
 * surface (`useOverlayLayer` / `hasOpenOverlay`) plus one end-to-end check
 * through `Overlay` for that rule.
 *
 * There was no dedicated test file for this module before; its coverage came
 * incidentally from other components' tests.
 */
import { act, createElement, StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { describe, expect, it } from "vitest";
import { renderHook } from "../../test/renderHook";
import { Overlay } from "./Overlay";
import { hasOpenOverlay, useOverlayLayer } from "./overlayStack";

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

describe("overlayStack", () => {
  it("starts closed and reports a non-top layer while the stack is empty", () => {
    expect(hasOpenOverlay()).toBe(false);

    // A closed layer registers nothing, and `isTop()` on an empty stack must be
    // false rather than throwing — that is the `stack.length > 0` guard.
    const closed = renderHook(() => useOverlayLayer(false));
    expect(hasOpenOverlay()).toBe(false);
    expect(closed.current.isTop()).toBe(false);
    closed.unmount();
  });

  it("registers a layer while open and removes it again on close", () => {
    const layer = renderHook(() => useOverlayLayer(true));
    expect(hasOpenOverlay()).toBe(true);
    expect(layer.current.isTop()).toBe(true);

    layer.unmount();
    expect(hasOpenOverlay()).toBe(false);
  });

  it("makes only the most recently opened layer the top one", () => {
    const outer = renderHook(() => useOverlayLayer(true));
    expect(outer.current.isTop()).toBe(true);

    const inner = renderHook(() => useOverlayLayer(true));
    expect(inner.current.isTop()).toBe(true);
    // The outer layer is still open but is no longer top — this is the rule
    // that stops a nested dialog's Escape from closing its parent too.
    expect(outer.current.isTop()).toBe(false);

    inner.unmount();
    expect(outer.current.isTop()).toBe(true);

    outer.unmount();
    expect(hasOpenOverlay()).toBe(false);
  });

  it("keeps the stack consistent across a StrictMode mount/unmount cycle", () => {
    // StrictMode runs effects twice on mount (setup -> cleanup -> setup). If the
    // cleanup did not unregister the layer, this would leak a permanent open
    // layer; the double-invoke is also the closest thing to a repeated cleanup
    // that React will do, so it is the natural place to look for `index === -1`.
    const container = document.createElement("div");
    document.body.appendChild(container);
    const root = createRoot(container);

    function Probe() {
      useOverlayLayer(true);
      return null;
    }

    act(() => {
      root.render(createElement(StrictMode, null, createElement(Probe)));
    });
    expect(hasOpenOverlay()).toBe(true);

    act(() => {
      root.unmount();
    });
    // Exactly zero layers remain — no leak from the extra setup pass.
    expect(hasOpenOverlay()).toBe(false);
  });

  it("does not leak layers when several open trees are unmounted in either order", () => {
    const a = renderHook(() => useOverlayLayer(true));
    const b = renderHook(() => useOverlayLayer(true));
    const c = renderHook(() => useOverlayLayer(true));
    expect(hasOpenOverlay()).toBe(true);

    // Unmount out of registration order: each cleanup must remove its own id.
    b.unmount();
    expect(c.current.isTop()).toBe(true);
    a.unmount();
    expect(c.current.isTop()).toBe(true);
    expect(hasOpenOverlay()).toBe(true);

    c.unmount();
    expect(hasOpenOverlay()).toBe(false);
  });

  it("lets only the topmost Overlay close on Escape", () => {
    const closed: string[] = [];

    function Pair({ showInner }: { showInner: boolean }) {
      return (
        <>
          <Overlay onClose={() => closed.push("outer")} open>
            <span>outer</span>
          </Overlay>
          {showInner
            ? <Overlay onClose={() => closed.push("inner")} open><span>inner</span></Overlay>
            : null}
        </>
      );
    }

    const container = document.createElement("div");
    document.body.appendChild(container);
    const root = createRoot(container);
    act(() => {
      root.render(createElement(Pair, { showInner: true }));
    });
    expect(hasOpenOverlay()).toBe(true);

    // Escape reaches both listeners; only the inner one may act on it.
    act(() => {
      window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    });
    expect(closed).toEqual(["inner"]);

    // With the inner overlay gone, the outer is top and answers Escape itself.
    act(() => {
      root.render(createElement(Pair, { showInner: false }));
    });
    act(() => {
      window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    });
    expect(closed).toEqual(["inner", "outer"]);

    act(() => {
      root.unmount();
    });
    expect(hasOpenOverlay()).toBe(false);
  });

  it("ignores non-Escape keys on the top overlay", () => {
    const closed: string[] = [];
    const container = document.createElement("div");
    document.body.appendChild(container);
    const root = createRoot(container);
    act(() => {
      root.render(createElement(Overlay, { children: createElement("span", null, "body"), onClose: () => closed.push("x"), open: true }));
    });
    act(() => {
      window.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter" }));
      window.dispatchEvent(new KeyboardEvent("keydown", { key: "a" }));
    });
    expect(closed).toEqual([]);

    act(() => {
      root.unmount();
    });
  });
});
