// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it } from "vitest";
import { activeScopes, isScopeActive, resetShortcutScopes, useShortcutScope } from "./scopes";

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

function mountScopes() {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  function Holder({ grid, zoom, vibe }: { grid: boolean; zoom: boolean; vibe: boolean }) {
    useShortcutScope("vibe", vibe);
    useShortcutScope("vibeGrid", grid);
    useShortcutScope("vibeZoom", zoom);
    return null;
  }
  return {
    close: () => {
      act(() => root.unmount());
      container.remove();
    },
    render: (props: { grid: boolean; zoom: boolean; vibe: boolean }) => act(() => root.render(<Holder {...props} />)),
  };
}

afterEach(() => {
  resetShortcutScopes();
});

describe("shortcut scopes", () => {
  it("starts with the global keymap only", () => {
    expect(activeScopes()).toEqual(["global"]);
    expect(isScopeActive("vibe")).toBe(false);
    expect(isScopeActive("global")).toBe(true);
  });

  it("puts a mode's scope innermost while it is held, and drops it on release", () => {
    const view = mountScopes();
    view.render({ grid: false, vibe: true, zoom: false });
    expect(activeScopes()).toEqual(["vibe", "global"]);
    view.close();
    expect(activeScopes()).toEqual(["global"]);
  });

  it("ranks scopes by their layer, not by who registered first", () => {
    // `vibe` is held for the panel's whole life while the grid layer comes and
    // goes with focus, so registration order must not decide precedence.
    const view = mountScopes();
    view.render({ grid: true, vibe: true, zoom: false });
    expect(activeScopes()).toEqual(["vibeGrid", "vibe", "global"]);
    view.render({ grid: true, vibe: true, zoom: true });
    // Both cannot really be on; if they are, the deeper layer still wins.
    expect(activeScopes()).toEqual(["vibeZoom", "vibeGrid", "vibe", "global"]);
    view.render({ grid: false, vibe: true, zoom: true });
    expect(activeScopes()).toEqual(["vibeZoom", "vibe", "global"]);
    view.close();
  });

  it("keeps a scope held while a second holder asks for it", () => {
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    function Pair({ both }: { both: boolean }) {
      useShortcutScope("vibe", true);
      useShortcutScope("vibe", both);
      return null;
    }
    act(() => root.render(<Pair both />));
    expect(activeScopes()).toEqual(["vibe", "global"]);
    // One holder releasing must not unlock the other.
    act(() => root.render(<Pair both={false} />));
    expect(activeScopes()).toEqual(["vibe", "global"]);
    act(() => root.unmount());
    container.remove();
    expect(activeScopes()).toEqual(["global"]);
  });

  it("releases a scope when its mode turns off, without touching the others", () => {
    const view = mountScopes();
    view.render({ grid: true, vibe: true, zoom: false });
    // The panel closes: its grid layer goes with it, not just its outer scope.
    view.render({ grid: false, vibe: false, zoom: false });
    expect(activeScopes()).toEqual(["global"]);
    view.close();
  });
});
