// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import { RemotePage } from "./RemotePage";

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

const roots: { root: ReturnType<typeof createRoot>; container: HTMLElement }[] = [];

afterEach(() => {
  for (const { root, container } of roots.splice(0)) {
    act(() => root.unmount());
    container.remove();
  }
});

function mount(autoConnectRemote: boolean, onToggleAutoConnectRemote: (value: boolean) => void) {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  roots.push({ container, root });
  act(() => {
    root.render(<RemotePage autoConnectRemote={autoConnectRemote} onToggleAutoConnectRemote={onToggleAutoConnectRemote} />);
  });
  return container;
}

describe("remotePage", () => {
  it("offers the launch toggle unchecked, with its description", () => {
    const container = mount(false, vi.fn());
    const toggle = container.querySelector<HTMLButtonElement>("[role=switch]")!;

    expect(toggle.getAttribute("aria-checked")).toBe("false");
    expect(toggle.getAttribute("aria-label")).toBe("Auto-connect phone on launch");
    expect(container.textContent).toContain("When a phone is already paired, connect to it automatically");
  });

  it("toggles the preference on and off as a controlled switch", () => {
    const onToggle = vi.fn();
    const container = mount(false, onToggle);
    const toggle = container.querySelector<HTMLButtonElement>("[role=switch]")!;

    act(() => toggle.click());
    expect(onToggle).toHaveBeenLastCalledWith(true);
    expect(toggle.getAttribute("aria-checked")).toBe("false"); // still controlled by the prop

    act(() => roots[0]!.root.render(<RemotePage autoConnectRemote onToggleAutoConnectRemote={onToggle} />));
    expect(toggle.getAttribute("aria-checked")).toBe("true");
    act(() => toggle.click());
    expect(onToggle).toHaveBeenLastCalledWith(false);
    expect(onToggle).toHaveBeenCalledTimes(2);
  });
});
