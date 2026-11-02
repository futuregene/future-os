// @vitest-environment jsdom
import type { VibeMenuRow } from "./menuNav";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { hasKeyboardOwner } from "../../lib/keyboardOwners";
import { firstEnabled, nextEnabled } from "./menuNav";
import { VibeMenu } from "./VibeMenu";

/**
 * The panel's menu: the remote control's only way into the panel's settings.
 *
 * Two halves, tested together because they are one behaviour: the pure walk
 * (`nextEnabled`) and the component that turns keys into it. The properties worth
 * pinning are the ones a remote depends on — it wraps, it skips a disabled row
 * instead of getting stuck, `←`/`→` adjust a value in place, and every key that can
 * open it can also close it.
 */
(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

function row(id: string, overrides: Partial<VibeMenuRow> = {}): VibeMenuRow {
  return { id, label: id, onRun: () => {}, ...overrides };
}

describe("nextEnabled", () => {
  const rows = [row("a"), row("b"), row("c")];

  it("steps forward and backward", () => {
    expect(nextEnabled(rows, 0, 1)).toBe(1);
    expect(nextEnabled(rows, 2, -1)).toBe(1);
  });

  it("wraps at both ends", () => {
    // Opposite of the grid's rule, deliberately: a menu is a short list where
    // "down at the bottom" has an obvious next.
    expect(nextEnabled(rows, 2, 1)).toBe(0);
    expect(nextEnabled(rows, 0, -1)).toBe(2);
  });

  it("skips a disabled row rather than getting stuck on it", () => {
    const withDisabled = [row("a"), row("b", { disabled: true }), row("c")];
    expect(nextEnabled(withDisabled, 0, 1)).toBe(2);
    expect(nextEnabled(withDisabled, 2, -1)).toBe(0);
  });

  it("stays put when every row is disabled", () => {
    const allOff = [row("a", { disabled: true }), row("b", { disabled: true })];
    expect(nextEnabled(allOff, 0, 1)).toBe(0);
  });

  it("survives an empty menu", () => {
    expect(nextEnabled([], 0, 1)).toBe(0);
    expect(firstEnabled([], 0)).toBe(0);
  });

  it("opens on the first row a reader can run", () => {
    expect(firstEnabled([row("a", { disabled: true }), row("b")], 0)).toBe(1);
  });
});

describe("vibeMenu", () => {
  let container: HTMLDivElement;
  let root: ReturnType<typeof createRoot>;

  /**
   * Press a key at the window, the way a remote control does: the panel listens
   * there (not on its own element), so focus cannot route a menu key away.
   */
  function key(init: KeyboardEventInit) {
    expect(container.querySelector("[role=\"menu\"]"), "the menu panel").not.toBeNull();
    act(() => {
      window.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, cancelable: true, ...init }));
    });
  }

  function selectedRow(): string | null {
    return container.querySelector<HTMLElement>("[aria-current=\"true\"]")?.dataset.vibeMenuRow ?? null;
  }

  function render(rows: VibeMenuRow[], onClose = () => {}) {
    act(() => root.render(<VibeMenu onClose={onClose} rows={rows} title="Panel menu" />));
  }

  beforeEach(() => {
    container = document.body.appendChild(document.createElement("div"));
    root = createRoot(container);
  });

  afterEach(() => {
    act(() => root.unmount());
    container.remove();
  });

  it("holds the keyboard while it is up", () => {
    // Registration is by presence, not focus: a click opens nothing here, and a
    // menu that did not stop the grid's arrows would move the ring behind it.
    expect(hasKeyboardOwner()).toBe(false);
    render([row("a")]);
    expect(hasKeyboardOwner()).toBe(true);
  });

  it("opens on the first row and moves with the arrows", () => {
    render([row("a"), row("b"), row("c")]);
    expect(selectedRow()).toBe("a");
    key({ code: "ArrowDown", key: "ArrowDown" });
    expect(selectedRow()).toBe("b");
    key({ code: "ArrowUp", key: "ArrowUp" });
    expect(selectedRow()).toBe("a");
    // Wrapping, so a remote never needs the opposite key to reach the last row.
    key({ code: "ArrowUp", key: "ArrowUp" });
    expect(selectedRow()).toBe("c");
  });

  it("runs the selected row on ⏎", () => {
    const onRun = vi.fn();
    render([row("a"), row("b", { onRun })]);
    key({ code: "ArrowDown", key: "ArrowDown" });
    key({ code: "Enter", key: "Enter" });
    expect(onRun).toHaveBeenCalledTimes(1);
  });

  it("does not run a disabled row", () => {
    const onRun = vi.fn();
    render([row("a", { disabled: true, onRun })]);
    key({ code: "Enter", key: "Enter" });
    expect(onRun).not.toHaveBeenCalled();
  });

  it("adjusts a value with ← and →, and leaves an action row alone", () => {
    const onAdjust = vi.fn();
    render([row("layout", { onAdjust, value: "自动" }), row("remove")]);
    key({ code: "ArrowRight", key: "ArrowRight" });
    key({ code: "ArrowLeft", key: "ArrowLeft" });
    expect(onAdjust.mock.calls).toEqual([[1], [-1]]);
    // Down to the action row: the horizontal keys now do nothing there.
    key({ code: "ArrowDown", key: "ArrowDown" });
    key({ code: "ArrowRight", key: "ArrowRight" });
    expect(onAdjust).toHaveBeenCalledTimes(2);
  });

  it("closes on Esc and on the remote's own menu button", () => {
    const onClose = vi.fn();
    render([row("a")], onClose);
    key({ code: "Escape", key: "Escape" });
    expect(onClose).toHaveBeenCalledTimes(1);
    // The key that opened it closes it: on a seven-key device, a menu whose own
    // button did not close it would be a trap.
    key({ code: "ContextMenu", key: "ContextMenu" });
    expect(onClose).toHaveBeenCalledTimes(2);
  });

  it("closes on a press outside it, without running a row", () => {
    const onClose = vi.fn();
    const onRun = vi.fn();
    render([row("a", { onRun })], onClose);
    const backdrop = container.querySelector<HTMLElement>("[role=\"presentation\"]")!;
    act(() => {
      backdrop.dispatchEvent(new MouseEvent("mousedown", { bubbles: true }));
    });
    expect(onClose).toHaveBeenCalledTimes(1);
    expect(onRun).not.toHaveBeenCalled();
  });

  it("reads a key that arrives without its physical code", () => {
    // A remote, a WebKit build and a synthetic press can all deliver only `key`.
    // Reading `code` alone left `←`/`→` doing nothing in the real page while every
    // jsdom test passed — the screenshot harness is what found it, and this pins the
    // fix where CI runs.
    const onAdjust = vi.fn();
    render([row("layout", { onAdjust, value: "自动" })]);
    act(() => {
      window.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, cancelable: true, key: "ArrowRight" }));
    });
    expect(onAdjust).toHaveBeenCalledWith(1);
  });

  it("shows a choice row's current value", () => {
    // "Which layout am I in" must be answerable without adjusting to find out.
    render([row("layout", { value: "3 列" })]);
    expect(container.textContent).toContain("3 列");
  });
  describe("the menu under a pointer", () => {
    function menuRow(id: string) {
      return container.querySelector<HTMLElement>(`[data-vibe-menu-row="${id}"]`)!;
    }

    it("runs the row that was clicked, and rings it first", () => {
      const onRun = vi.fn();
      render([row("a"), row("b", { onRun })]);
      const target = menuRow("b");

      act(() => {
      // React derives `onMouseEnter` from a `mouseover` that crossed the boundary.
        target.dispatchEvent(new MouseEvent("mouseover", { bubbles: true }));
      });
      // Hovering is what a pointer's "selection" is: the ring follows the hand, so the
      // keyboard picks up from the row the pointer last touched.
      expect(selectedRow()).toBe("b");

      act(() => target.click());
      expect(onRun).toHaveBeenCalledTimes(1);
    });

    it("does not run a disabled row that was clicked", () => {
    // `aria-disabled` keeps the button pressable so the row can still be read; the
    // guard is what makes it inert.
      const onRun = vi.fn();
      render([row("a", { disabled: true, onRun })]);

      act(() => menuRow("a").click());

      expect(onRun).not.toHaveBeenCalled();
    });

    it("keeps a press on the panel away from the backdrop's dismiss", () => {
    // The backdrop closes the menu; a press on the sheet itself is reading, not
    // dismissing, and without the stop it would close under the reader's finger.
      const onClose = vi.fn();
      render([row("a")], onClose);

      act(() => {
        menuRow("a").dispatchEvent(new MouseEvent("mousedown", { bubbles: true }));
      });

      expect(onClose).not.toHaveBeenCalled();
    });

    it("leaves a key another handler already took alone", () => {
    // A press that was answered before it reached the window (the panel's own chord, an
    // input method committing) must not also move this menu.
      render([row("a"), row("b")]);
      const taken = new KeyboardEvent("keydown", { bubbles: true, cancelable: true, code: "ArrowDown", key: "ArrowDown" });
      taken.preventDefault();
      act(() => {
        window.dispatchEvent(taken);
      });

      expect(selectedRow()).toBe("a");
    });

    it("leaves a chord to the panel, even one of its own keys", () => {
    // `⌘⌥→` is the panel's "move this cell": the menu holds the keyboard, but it must
    // not also adjust the row the ring is on — one press, one action.
      const onAdjust = vi.fn();
      render([row("layout", { onAdjust })]);
      act(() => {
        window.dispatchEvent(new KeyboardEvent("keydown", {
          altKey: true,
          bubbles: true,
          cancelable: true,
          code: "ArrowRight",
          ctrlKey: true,
          key: "ArrowRight",
        }));
      });

      expect(onAdjust).not.toHaveBeenCalled();
      expect(selectedRow()).toBe("layout");
    });

    it("ignores a key it does not answer", () => {
    // Letters belong to nothing here: the menu consumes the keys it knows and lets the
    // rest through untouched (the editor two layers down may still want them).
      render([row("a"), row("b")]);
      const letter = new KeyboardEvent("keydown", { bubbles: true, cancelable: true, code: "KeyA", key: "a" });
      let reached = false;
      act(() => {
        reached = window.dispatchEvent(letter);
      });

      expect(reached).toBe(true);
      expect(selectedRow()).toBe("a");
    });
  });
});
