// @vitest-environment jsdom
import type { ReactNode } from "react";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { describe, expect, it } from "vitest";
import { HelpTip } from "./HelpTip";

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

function mount(node: ReactNode) {
  const container = document.createElement("div");
  document.body.appendChild(container);
  const root = createRoot(container);
  act(() => root.render(node));
  return {
    container,
    unmount: () => {
      act(() => root.unmount());
      container.remove();
    },
  };
}

function trigger(container: HTMLElement) {
  return container.querySelector<HTMLButtonElement>("button");
}

/** The panel is portalled to `document.body`, so it is not inside `container`. */
function panel() {
  return document.querySelector<HTMLElement>("[role=note]");
}

describe("help tip", () => {
  it("stays closed until the icon is clicked, and carries no hover tooltip", () => {
    const { container, unmount } = mount(
      <HelpTip label="About idle days">Idle means nothing new happened.</HelpTip>,
    );
    try {
      const button = trigger(container)!;
      expect(button.getAttribute("aria-expanded")).toBe("false");
      // A `title` would put the native hover popup back on top of the click one.
      expect(button.getAttribute("title")).toBeNull();
      expect(panel()).toBeNull();
      expect(document.body.textContent).not.toContain("Idle means nothing new happened.");
    }
    finally {
      unmount();
    }
  });

  it("opens on click, labels the panel as a note, and closes on a second click", () => {
    const { container, unmount } = mount(
      <HelpTip label="About idle days">Idle means nothing new happened.</HelpTip>,
    );
    try {
      const button = trigger(container)!;
      act(() => button.click());
      expect(button.getAttribute("aria-expanded")).toBe("true");
      const note = panel()!;
      expect(note.textContent).toBe("Idle means nothing new happened.");
      // `aria-controls` has to point at the panel that is actually on screen.
      expect(note.id).toBe(button.getAttribute("aria-controls"));

      act(() => button.click());
      expect(panel()).toBeNull();
      expect(button.getAttribute("aria-expanded")).toBe("false");
    }
    finally {
      unmount();
    }
  });

  it("closes on a click outside and on Escape", () => {
    const { container, unmount } = mount(
      <HelpTip label="About idle days">Idle means nothing new happened.</HelpTip>,
    );
    try {
      const button = trigger(container)!;

      act(() => button.click());
      expect(panel()).not.toBeNull();
      act(() => {
        document.body.dispatchEvent(new MouseEvent("pointerdown", { bubbles: true }));
      });
      expect(panel()).toBeNull();

      act(() => button.click());
      expect(panel()).not.toBeNull();
      act(() => {
        document.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, key: "Escape" }));
      });
      expect(panel()).toBeNull();
    }
    finally {
      unmount();
    }
  });
});
