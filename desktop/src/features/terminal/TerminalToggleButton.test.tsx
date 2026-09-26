// @vitest-environment jsdom
import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { TerminalToggleButton } from "./TerminalToggleButton";

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

let container: HTMLDivElement;
let root: ReturnType<typeof createRoot>;

async function mount(props: { open: boolean; onToggle: () => void; shortcut?: string }) {
  await act(async () => {
    root.render(createElement(TerminalToggleButton, { shortcut: "Ctrl+J", ...props }));
  });
}

function button(): HTMLButtonElement {
  return container.querySelector<HTMLButtonElement>("button[data-component='terminal-toggle']")!;
}

beforeEach(() => {
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

describe("terminalToggleButton", () => {
  it("describes opening the panel, with the shortcut, while it is closed", async () => {
    await mount({ onToggle: () => {}, open: false });
    expect(button().getAttribute("aria-label")).toBe("Show terminal (Ctrl+J)");
    expect(button().getAttribute("aria-pressed")).toBe("false");
    expect(button().getAttribute("title")).toBe("Show terminal (Ctrl+J)");
  });

  it("describes closing it while it is open", async () => {
    await mount({ onToggle: () => {}, open: true });
    expect(button().getAttribute("aria-label")).toBe("Hide terminal (Ctrl+J)");
    expect(button().getAttribute("aria-pressed")).toBe("true");
  });

  it("uses the platform's shortcut label verbatim", async () => {
    await mount({ onToggle: () => {}, open: true, shortcut: "⇧⌘J" });
    expect(button().getAttribute("aria-label")).toBe("Hide terminal (⇧⌘J)");
  });

  it("calls back exactly once per activation", async () => {
    const onToggle = vi.fn();
    await mount({ onToggle, open: false });
    await act(async () => {
      button().click();
    });
    expect(onToggle).toHaveBeenCalledTimes(1);
  });
});
