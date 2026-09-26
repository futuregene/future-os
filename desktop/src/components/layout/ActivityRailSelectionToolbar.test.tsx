// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { describe, expect, it, vi } from "vitest";
import { ActivityRailSelectionToolbar } from "./ActivityRailSelectionToolbar";

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

function mount(props: { selectedCount: number; totalCount: number }) {
  const handlers = { onCancel: vi.fn(), onDelete: vi.fn(), onToggleAll: vi.fn() };
  const container = document.createElement("div");
  document.body.appendChild(container);
  const root = createRoot(container);
  act(() => root.render(<ActivityRailSelectionToolbar {...handlers} {...props} />));
  return {
    checkbox: () => container.querySelector<HTMLInputElement>("input[type=\"checkbox\"]")!,
    container,
    handlers,
    label: () => container.querySelector("label span")!.textContent,
    rerender: (next: { selectedCount: number; totalCount: number }) =>
      act(() => root.render(<ActivityRailSelectionToolbar {...handlers} {...next} />)),
    unmount: () => {
      act(() => root.unmount());
      container.remove();
    },
  };
}

function button(container: HTMLElement, label: string) {
  return [...container.querySelectorAll<HTMLButtonElement>("button")].find(item => item.getAttribute("aria-label") === label)!;
}

describe("activity rail selection toolbar", () => {
  it("exposes a labelled toolbar with a select-all checkbox, delete and cancel", () => {
    const view = mount({ selectedCount: 0, totalCount: 2 });
    const toolbar = view.container.querySelector("[role=\"toolbar\"]")!;
    expect(toolbar.getAttribute("aria-label")).toBe("Select chats");
    expect(toolbar.getAttribute("data-activity-rail-selection-toolbar")).toBe("true");
    expect(button(view.container, "Delete")).toBeTruthy();
    expect(button(view.container, "Cancel")).toBeTruthy();
    view.unmount();
  });

  // The toolbar's state is the (selected, total) pair. Every arm is asserted:
  // a 100%-line / 62.5%-branch reading meant only some pairs had ever rendered.
  it.each([
    { expectedChecked: false, expectedIndeterminate: false, label: "Select all", selectedCount: 0, totalCount: 0 },
    { expectedChecked: false, expectedIndeterminate: false, label: "Select all", selectedCount: 0, totalCount: 3 },
    { expectedChecked: false, expectedIndeterminate: true, label: "2 selected", selectedCount: 2, totalCount: 3 },
    { expectedChecked: true, expectedIndeterminate: false, label: "3 selected", selectedCount: 3, totalCount: 3 },
  ])(
    "$selectedCount of $totalCount ⇒ checked=$expectedChecked indeterminate=$expectedIndeterminate",
    ({ expectedChecked, expectedIndeterminate, label, selectedCount, totalCount }) => {
      const view = mount({ selectedCount, totalCount });
      expect(view.checkbox().checked).toBe(expectedChecked);
      expect(view.checkbox().indeterminate).toBe(expectedIndeterminate);
      expect(view.label()).toBe(label);
      view.unmount();
    },
  );

  it("never claims a full selection over an empty scope", () => {
    // 0 of 0 is "nothing selected", not "everything is selected" — the
    // `totalCount > 0` guard is what separates those two readings.
    const view = mount({ selectedCount: 0, totalCount: 0 });
    expect(view.checkbox().checked).toBe(false);
    expect(view.label()).toBe("Select all");
    view.unmount();
  });

  it("recomputes the tri-state as the selection changes", () => {
    const view = mount({ selectedCount: 0, totalCount: 4 });
    expect(view.checkbox().indeterminate).toBe(false);

    view.rerender({ selectedCount: 1, totalCount: 4 });
    expect(view.checkbox().indeterminate).toBe(true);
    expect(view.checkbox().checked).toBe(false);
    expect(view.label()).toBe("1 selected");

    view.rerender({ selectedCount: 4, totalCount: 4 });
    expect(view.checkbox().indeterminate).toBe(false);
    expect(view.checkbox().checked).toBe(true);

    view.rerender({ selectedCount: 0, totalCount: 4 });
    expect(view.checkbox().indeterminate).toBe(false);
    expect(view.checkbox().checked).toBe(false);
    expect(view.label()).toBe("Select all");
    view.unmount();
  });

  it("clears the indeterminate flag when the scope itself grows to include every selection", () => {
    const view = mount({ selectedCount: 2, totalCount: 4 });
    expect(view.checkbox().indeterminate).toBe(true);
    // Two threads were archived away by another client, so the selection is
    // suddenly the whole scope: the box must become a plain checked one.
    view.rerender({ selectedCount: 2, totalCount: 2 });
    expect(view.checkbox().indeterminate).toBe(false);
    expect(view.checkbox().checked).toBe(true);
    view.unmount();
  });

  it("toggles all from the checkbox and never calls delete with nothing selected", () => {
    const view = mount({ selectedCount: 0, totalCount: 3 });
    const remove = button(view.container, "Delete");
    expect(remove.disabled).toBe(true);
    act(() => remove.click());
    expect(view.handlers.onDelete).not.toHaveBeenCalled();

    act(() => view.checkbox().click());
    expect(view.handlers.onToggleAll).toHaveBeenCalledTimes(1);
    view.unmount();
  });

  it("enables delete and cancels selection mode from its own buttons", () => {
    const view = mount({ selectedCount: 1, totalCount: 3 });
    const remove = button(view.container, "Delete");
    expect(remove.disabled).toBe(false);
    act(() => remove.click());
    expect(view.handlers.onDelete).toHaveBeenCalledTimes(1);

    act(() => button(view.container, "Cancel").click());
    expect(view.handlers.onCancel).toHaveBeenCalledTimes(1);
    expect(view.handlers.onToggleAll).not.toHaveBeenCalled();
    view.unmount();
  });

  it("labels both action buttons with their icon-plus-text affordance", () => {
    const view = mount({ selectedCount: 1, totalCount: 3 });
    for (const [label, icon] of [["Delete", "lucide-trash-2"], ["Cancel", "lucide-x"]] as const) {
      const action = button(view.container, label);
      expect(action.getAttribute("title")).toBe(label);
      expect(action.querySelector(`svg.${icon}`)).not.toBeNull();
      expect(action.querySelector(".activity-rail-selection-action-label")!.textContent).toBe(label);
    }
    view.unmount();
  });
});
