// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { RemoteSlashMenu } from "./RemoteSlashMenu";

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

/**
 * The `/` menu a remote composer opens.
 *
 * Above the box rather than below it: the box sits at the foot of the window, so
 * a menu opening downward would open off-screen.
 */
async function mount(activeIndex = 0) {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const onChoose = vi.fn();
  const command = { description: "", id: "compact", label: "Compact context", run: vi.fn() };
  await act(async () => {
    root.render(
      <RemoteSlashMenu activeIndex={activeIndex} groups={{ commands: [command] }} onChoose={onChoose} />,
    );
  });
  return {
    command,
    container,
    onChoose,
    item: () => container.querySelector<HTMLButtonElement>("[role='option']")!,
    unmount: async () => {
      await act(async () => root.unmount());
      container.remove();
    },
  };
}

it("lists the host commands and reports the chosen one", async () => {
  const view = await mount();
  expect(view.container.querySelector("[role='listbox']")).not.toBeNull();
  expect(view.container.textContent).toContain("Compact context");
  await act(async () => view.item().click());
  expect(view.onChoose).toHaveBeenCalledWith(view.command);
  await view.unmount();
});

/** The keyboard's position is the one the menu draws as chosen. */
it("marks the active row", async () => {
  const first = await mount(0);
  expect(first.item().getAttribute("aria-selected")).toBe("true");
  await first.unmount();

  const second = await mount(1);
  expect(second.item().getAttribute("aria-selected")).toBe("false");
  await second.unmount();
});

/** Nothing to offer means nothing drawn — not an empty panel. */
it("draws nothing when no command matches", async () => {
  const container = document.createElement("div");
  const root = createRoot(container);
  await act(async () => {
    root.render(<RemoteSlashMenu activeIndex={0} groups={{ commands: [] }} onChoose={vi.fn()} />);
  });
  expect(container.querySelector("[role='listbox']")).toBeNull();
  await act(async () => root.unmount());
});
