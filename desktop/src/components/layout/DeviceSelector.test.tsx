// @vitest-environment jsdom
import type { DeviceFilter } from "../../features/remote-peer/mergeConversations";
import type { RemotePeer } from "../../features/remote-peer/remotePeerClient";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { beforeEach, expect, it, vi } from "vitest";
import { DeviceSelector } from "./DeviceSelector";

/**
 * The collapsed button and the expanded entries must show the same picture for
 * the same choice. They did not: "all devices" collapsed to a 🗂 while the entry
 * that selects it showed a monitor, so choosing it looked like nothing happened.
 */

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

const peer: RemotePeer = {
  agentAvailable: true,
  bridgeInstanceId: "bridge_1",
  connected: true,
  desktopId: "desktop_a",
  error: null,
  features: [],
  icon: "laptop",
  name: "Studio iMac",
  pairId: "pair_1",
};

beforeEach(() => {
  document.body.innerHTML = "";
});

async function mount(filter: DeviceFilter) {
  const onChange = vi.fn();
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  await act(async () => {
    root.render(
      <DeviceSelector filter={filter} onChange={onChange} onManage={() => {}} peers={[peer]} />,
    );
  });
  return { container, onChange, root };
}

/** The lucide icon name a node renders, which is what "the same picture" means. */
function iconName(node: Element | null): string | null {
  const svg = node?.querySelector("svg");
  const names = [...(svg?.classList ?? [])].filter(name => name.startsWith("lucide-") && name !== "lucide");
  return names[0] ?? null;
}

it("shows the same icon for the selection as for the entry that selects it", async () => {
  for (const [filter, expected] of [
    [{ kind: "all" } as DeviceFilter, "lucide-monitor-smartphone"],
    [{ kind: "device", desktopId: null } as DeviceFilter, "lucide-monitor"],
  ] as const) {
    const view = await mount(filter);
    const button = view.container.querySelector("button[aria-haspopup='listbox']")!;
    expect(iconName(button)).toBe(expected);

    await act(async () => {
      button.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });
    const selected = view.container.querySelector("[role='option'][aria-selected='true']");
    expect(iconName(selected)).toBe(iconName(button));
    await view.root.unmount();
  }
});

/**
 * The entry carries the machine's name as its accessible name.
 *
 * Its visible name starts with the machine's decorative glyph, so without this
 * the control is announced (and matched) as an emoji followed by a name — the
 * same defect the collapsed button had.
 */
it("gives each entry the machine's name, not its glyph", async () => {
  const view = await mount({ kind: "all" });
  const button = view.container.querySelector("button[aria-haspopup='listbox']")!;
  await act(async () => {
    button.dispatchEvent(new MouseEvent("click", { bubbles: true }));
  });

  const named = [...view.container.querySelectorAll("[role='option']")]
    .map(node => node.getAttribute("aria-label"));
  expect(named).toContain("Studio iMac");
  await view.root.unmount();
});
