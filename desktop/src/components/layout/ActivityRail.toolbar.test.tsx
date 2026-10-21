// @vitest-environment jsdom
import type { ComponentProps } from "react";
import type { StoredWorkspace } from "../../integrations/storage/threadStore";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { flushAsync } from "../../test/renderHook";
import { ActivityRail } from "./ActivityRail";

vi.mock("../../integrations/skills/skillsClient", () => ({ listInstalledSkills: async () => [] }));
vi.mock("../../integrations/agent/agentStateCache", () => ({ useCachedAgentState: () => undefined }));
vi.mock("../../lib/useIsFullscreen", () => ({ useIsFullscreen: () => false }));
vi.mock("./hooks/usePendingApprovalCounts", () => ({ usePendingApprovalCounts: () => new Map() }));
vi.mock("../../lib/useFloatingScrollbar", () => ({
  useFloatingScrollbar: () => ({ scrollRef: { current: null }, scrollbar: { height: 0, top: 0, visible: false }, handleScroll: () => {}, handleThumbPointerDown: () => {} }),
}));

/**
 * The rows above the rail's list, and what is *not* in them.
 *
 * The complaint these answer is about what is drawn where, so each reads
 * container relationships — which row an element is inside, what else is in it —
 * rather than a label. A behaviour test that reads the list stays green with a
 * control moved back into the wrong row; these do not.
 */

function thread(id: string, title: string) {
  return {
    id,
    agentSessionId: id,
    title,
    mode: "chat" as const,
    workspaceId: "ws",
    status: "active" as const,
    pinned: false,
    readonly: false,
    createdAt: 0,
    updatedAt: 0,
  };
}

function props(): ComponentProps<typeof ActivityRail> {
  return {
    active: "chat",
    expanded: true,
    activeThreadId: null,
    threads: [thread("a", "Alpha chat"), thread("b", "Beta chat")],
    threadRunStatuses: {},
    threadStreamingStatuses: {},
    unreadThreadIds: new Set(),
    workspaces: [] as StoredWorkspace[],
    skillIntroDismissed: true,
    onDismissSkillIntro: vi.fn(),
    onChange: vi.fn(),
    onBatchDeleteThreads: vi.fn(),
    onDeleteRemoteConversations: vi.fn(),
    onRemoteConversationDeleted: vi.fn(),
    onDeleteThread: vi.fn(),
    onNewChat: vi.fn(),
    onOpenModels: vi.fn(),
    onNewWorkspace: vi.fn(),
    onRenameThread: vi.fn(),
    onRenameWorkspace: vi.fn(),
    onDeleteWorkspace: vi.fn(),
    onRestoreThread: vi.fn(),
    onSelectWorkspace: vi.fn(),
    onSelectThread: vi.fn(),
    onTogglePinThread: vi.fn(),
    onTogglePinWorkspace: vi.fn(),
    onToggleExpanded: vi.fn(),
  };
}

const peer = {
  agentAvailable: true,
  bridgeInstanceId: "bridge_1",
  connected: true,
  stopped: false,
  desktopId: "desktop_a",
  error: null,
  features: [] as string[],
  icon: "laptop",
  name: "Studio iMac",
  pairId: "pair_1",
};

async function render(p: ComponentProps<typeof ActivityRail>) {
  const container = document.createElement("div");
  document.body.appendChild(container);
  const root = createRoot(container);
  act(() => root.render(<ActivityRail {...p} />));
  await flushAsync();
  return { container, root };
}

function band(container: HTMLElement, id: "rail-view-band" | "rail-list-tools") {
  return container.querySelector<HTMLElement>(`[data-testid="${id}"]`);
}

function searchGlyph(root: HTMLElement) {
  return root.querySelector<HTMLButtonElement>("button[aria-label='Search conversations']");
}
function searchField(root: HTMLElement) {
  return root.querySelector<HTMLInputElement>("input[aria-label='Search conversations']");
}

describe("the rail's rows above the list", () => {
  beforeEach(() => {
    localStorage.clear();
    document.body.innerHTML = "";
  });

  /** The switch's own row holds the switch, and nothing else — in both lists. */
  it.each(["workspace", "recent"] as const)("keeps the switch's row to the switch (%s)", async (view) => {
    const { container } = await render(props());
    if (view === "recent")
      act(() => container.querySelector<HTMLButtonElement>("[data-testid='rail-view-recent']")!.click());
    await flushAsync();

    const viewBand = band(container, "rail-view-band")!;
    expect(viewBand.querySelector("[data-testid='rail-view-switch']")).not.toBeNull();
    expect(viewBand.querySelector("input")).toBeNull();
    expect(viewBand.querySelector("button[aria-haspopup='menu']")).toBeNull();
  });

  /** Two lists, in this order, and no third. */
  it("offers the workspace list and the recent list, and nothing else", async () => {
    const { container } = await render(props());
    const tabs = [...container.querySelectorAll<HTMLButtonElement>("[data-testid='rail-view-switch'] button")];
    expect(tabs.map(tab => tab.getAttribute("data-testid"))).toEqual(["rail-view-workspace", "rail-view-recent"]);
    expect(tabs.map(tab => tab.textContent?.trim())).toEqual(["Workspaces", "Recent"]);
  });

  /**
   * The search is one glyph until it is clicked.
   *
   * The rail is a narrow column and the list is the point; a field drawn all the
   * time spends a row on a control most sessions never touch. So the row holds a
   * magnifier, and the field appears in its place on a click.
   */
  it("draws a magnifier and no field, until the magnifier is clicked", async () => {
    const { container } = await render(props());

    expect(searchField(container)).toBeNull();
    const glyph = searchGlyph(container)!;
    expect(glyph).not.toBeNull();

    act(() => glyph.click());
    await flushAsync();

    const field = searchField(container);
    expect(field).not.toBeNull();
    expect(field!.value).toBe("");
    // It took the glyph's place in the same row rather than pushing a new one.
    expect(band(container, "rail-list-tools")!.contains(field)).toBe(true);
  });

  it("closes the field, and clears the filter with it", async () => {
    const { container } = await render({ ...props(), active: "chat" });
    act(() => container.querySelector<HTMLButtonElement>("button[aria-label='Search conversations']")!.click());
    await flushAsync();

    const input = searchField(container)!;
    await act(async () => {
      const descriptor = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value");
      descriptor?.set?.call(input, "Alpha");
      input.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await flushAsync();
    const list = container.querySelector("[data-activity-rail-content]")!;
    expect(list.textContent).not.toContain("Beta chat");

    act(() => container.querySelector<HTMLButtonElement>("button[aria-label='Close search']")!.click());
    await flushAsync();

    // No field, and no forgotten filter: the whole list is back.
    expect(searchField(container)).toBeNull();
    expect(list.textContent).toContain("Beta chat");
  });

  /** The machine picker stays beside the glyph, and beside the open field. */
  it("keeps the machine picker in the search's own row", async () => {
    const p = props();
    p.remotePeers = [peer];
    p.onChangeDeviceFilter = vi.fn();
    p.onManageDesktops = vi.fn();
    const { container } = await render(p);

    const tools = band(container, "rail-list-tools")!;
    const picker = () => container.querySelector("button[aria-label='All devices']");
    expect(tools.contains(picker())).toBe(true);
    expect(tools.contains(searchGlyph(container))).toBe(true);
    // Compact: glyph, name and chevron in a fraction of the row.
    expect(picker()!.className).toContain("max-w-30");

    act(() => searchGlyph(container)!.click());
    await flushAsync();
    expect(tools.contains(picker())).toBe(true);
    expect(tools.contains(searchField(container))).toBe(true);
  });

  /**
   * Nothing above the list offers a batch or a create action.
   *
   * Those belong to what they act on: "new workspace" and "new conversation" sit
   * on the section headings inside the list, and a batch is entered from a
   * heading's own menu. A `…` up here marked "the whole list", which is a
   * selection with no boundary a reader can point at.
   */
  it.each(["workspace", "recent"] as const)("draws no create control and no menu above the list (%s)", async (view) => {
    const p = props();
    p.remotePeers = [peer];
    p.onChangeDeviceFilter = vi.fn();
    p.onManageDesktops = vi.fn();
    const { container } = await render(p);
    if (view === "recent")
      act(() => container.querySelector<HTMLButtonElement>("[data-testid='rail-view-recent']")!.click());
    await flushAsync();

    for (const row of [band(container, "rail-view-band")!, band(container, "rail-list-tools")!]) {
      expect(row.querySelector("button[aria-haspopup='menu']")).toBeNull();
      expect(row.querySelector("button[aria-label='New workspace']")).toBeNull();
      expect(row.querySelector("button[aria-label='New chat']")).toBeNull();
    }
    expect(container.querySelector("button[aria-label='Recent list actions']")).toBeNull();
  });

  /**
   * A batch is entered from inside the list, which is still the only way in.
   *
   * The workspace and chat headings keep their own menus; removing the one above
   * the list must not have removed those with it.
   */
  it("keeps a heading's own menu for a batch", async () => {
    const p = props();
    p.threads = [thread("a", "Alpha chat")];
    const { container } = await render(p);

    expect(container.querySelector("button[aria-label='Chat section actions']")).not.toBeNull();
  });
});
