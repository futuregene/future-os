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
  stopped: false,
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

async function mount(filter: DeviceFilter, peers: RemotePeer[] = [peer]) {
  const onChange = vi.fn();
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  await act(async () => {
    root.render(
      <DeviceSelector filter={filter} onChange={onChange} onManage={() => {}} peers={peers} />,
    );
  });
  return { container, onChange, root };
}

/** Open the picker and read each machine's entry note, by machine id. */
async function notes(peers: RemotePeer[]): Promise<Record<string, string>> {
  const view = await mount({ kind: "all" }, peers);
  const button = view.container.querySelector("button[aria-haspopup='listbox']")!;
  await act(async () => {
    button.dispatchEvent(new MouseEvent("click", { bubbles: true }));
  });
  const found: Record<string, string> = {};
  for (const peer of peers) {
    const row = [...view.container.querySelectorAll<HTMLElement>("[role='option']")]
      .find(node => node.getAttribute("aria-label") === (peer.name ?? peer.desktopId));
    found[peer.desktopId] = row?.querySelector<HTMLElement>("[data-testid='option-note']")?.textContent ?? "";
  }
  await view.root.unmount();
  return found;
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

/**
 * A machine that is not answering is listed with *which* kind of not-answering
 * it is: still being reached, unreachable, or switched off by the user. The
 * picker is where the user goes to ask why a machine's conversations are
 * missing from the merged list, so "Disconnected" for all three would be an
 * answer that explains nothing.
 */
it("says why each machine is not in the list", async () => {
  const found = await notes([
    { ...peer, desktopId: "desktop_a", name: "Live" },
    { ...peer, connected: false, desktopId: "desktop_b", name: "Reaching" },
    { ...peer, connected: false, desktopId: "desktop_c", error: "gone", name: "Gone" },
    { ...peer, connected: false, desktopId: "desktop_d", name: "Mine", stopped: true },
  ]);

  // A machine that answers needs no note: its own rows are in the list.
  expect(found.desktop_a).toBe("");
  expect(found.desktop_b).toBe("Connecting…");
  expect(found.desktop_c).toBe("Offline");
  expect(found.desktop_d).toBe("Stopped");
});

/** Open the picker on a mutable view, so each case can click an entry. */
async function openPicker(peers: RemotePeer[] = [peer]) {
  const onManage = vi.fn();
  const onChange = vi.fn();
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  await act(async () => {
    root.render(
      <DeviceSelector filter={{ kind: "all" }} onChange={onChange} onManage={onManage} peers={peers} />,
    );
  });
  const trigger = container.querySelector<HTMLButtonElement>("button[aria-haspopup='listbox']")!;
  await act(async () => {
    trigger.dispatchEvent(new MouseEvent("click", { bubbles: true }));
  });
  const option = (label: string) =>
    [...container.querySelectorAll<HTMLElement>("[role='option']")]
      .find(node => node.getAttribute("aria-label") === label)!;
  return { container, onChange, onManage, option, root, trigger };
}

it("applies the filter each entry stands for, and closes the picker", async () => {
  const view = await openPicker();
  try {
    const options = [...view.container.querySelectorAll<HTMLElement>("[role='option']")];
    // Every machine on screen, this one included — plus the "all" entry.
    expect(options.map(node => node.getAttribute("aria-label")))
      .toEqual(["All devices", "This computer", "Studio iMac"]);

    await act(async () => view.option("All devices").click());
    expect(view.onChange).toHaveBeenLastCalledWith({ kind: "all" });
    expect(view.container.querySelector("[role='listbox']")).toBeNull();
    expect(view.trigger.getAttribute("aria-expanded")).toBe("false");

    // Re-open for the next entry: the picker is a choice, not a toggle left on.
    await act(async () => view.trigger.dispatchEvent(new MouseEvent("click", { bubbles: true })));
    await act(async () => view.option("This computer").click());
    expect(view.onChange).toHaveBeenLastCalledWith({ kind: "device", desktopId: null });

    await act(async () => view.trigger.dispatchEvent(new MouseEvent("click", { bubbles: true })));
    await act(async () => view.option("Studio iMac").click());
    expect(view.onChange).toHaveBeenLastCalledWith({ kind: "device", desktopId: "desktop_a" });
    expect(view.container.querySelector("[role='listbox']")).toBeNull();
  }
  finally {
    await act(async () => view.root.unmount());
  }
});

it("hands off to the manage screen and closes the picker behind it", async () => {
  const view = await openPicker();
  try {
    const manage = [...view.container.querySelectorAll<HTMLButtonElement>("button")]
      .find(button => button.textContent === "Manage desktops…")!;
    await act(async () => manage.click());

    expect(view.onManage).toHaveBeenCalledTimes(1);
    expect(view.container.querySelector("[role='listbox']")).toBeNull();
  }
  finally {
    await act(async () => view.root.unmount());
  }
});

it("closes the picker when the click lands outside it", async () => {
  const view = await openPicker();
  try {
    expect(view.container.querySelector("[role='listbox']")).not.toBeNull();
    await act(async () => {
      document.body.dispatchEvent(new MouseEvent("pointerdown", { bubbles: true }));
    });
    expect(view.container.querySelector("[role='listbox']")).toBeNull();
    expect(view.trigger.getAttribute("aria-expanded")).toBe("false");
  }
  finally {
    await act(async () => view.root.unmount());
  }
});
