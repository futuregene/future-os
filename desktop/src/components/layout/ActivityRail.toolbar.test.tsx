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

  /**
   * The search's row is **above** the switch's row.
   *
   * Reported as "why is the search under workspaces/recent — shouldn't it be
   * above?". It was: the filter row has always sat directly over the list it
   * filters, with the switch under it. A revert put the two rows back in the
   * other order and nothing noticed, because every test read one row at a time.
   * So this reads their *order* in the document, which is the only shape that
   * goes red when they are swapped.
   */
  it("puts the search's row above the switch's row", async () => {
    const p = props();
    p.remotePeers = [peer];
    p.onChangeDeviceFilter = vi.fn();
    p.onManageDesktops = vi.fn();
    const { container } = await render(p);

    const searchRow = band(container, "rail-list-tools")!;
    const switchRow = band(container, "rail-view-band")!;
    expect(searchRow).not.toBeNull();
    expect(switchRow).not.toBeNull();

    // The search row comes first: the switch follows it in the document.
    expect(searchRow.compareDocumentPosition(switchRow) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    // Both are above the list they describe.
    const list = container.querySelector("[data-activity-rail-scroll]")!;
    expect(switchRow.compareDocumentPosition(list) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    // And the search itself is in that top row, not beside the switch.
    expect(searchRow.querySelector("button[aria-label='Search conversations']")).not.toBeNull();
    expect(switchRow.querySelector("button[aria-label='Search conversations']")).toBeNull();
  });

  /** The same order with no machine paired, where the row holds only the search. */
  it("keeps that order when the search is the row's only control", async () => {
    const { container } = await render(props());
    const searchRow = band(container, "rail-list-tools")!;
    const switchRow = band(container, "rail-view-band")!;
    expect(searchRow.children).toHaveLength(1);
    expect(searchRow.compareDocumentPosition(switchRow) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
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
   * The switch's order is the reader's, not a constant.
   *
   * Exchanging the two segments is only worth offering if it survives a restart,
   * and the rail is what has to read the stored order back and draw it — the
   * hook alone remembering it would still show the default here.
   */
  it("draws the switch in the order the reader left it", async () => {
    localStorage.setItem("future.activityRail.viewOrder", JSON.stringify(["recent", "workspace"]));
    const { container } = await render(props());
    const tabs = [...container.querySelectorAll<HTMLButtonElement>("[data-testid='rail-view-switch'] button")];
    expect(tabs.map(tab => tab.getAttribute("data-testid"))).toEqual(["rail-view-recent", "rail-view-workspace"]);
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

    // One `×` answers the state of the field: with a query it clears, and with
    // an empty field it closes. Closing directly (rather than clearing first) is
    // Escape, or a second press of the same control — both covered below.
    act(() => input.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true })));
    await flushAsync();

    // No field, and no forgotten filter: the whole list is back.
    expect(searchField(container)).toBeNull();
    expect(list.textContent).toContain("Beta chat");

    // And the same close, from the empty field's `×`.
    act(() => container.querySelector<HTMLButtonElement>("button[aria-label='Search conversations']")!.click());
    await flushAsync();
    act(() => container.querySelector<HTMLButtonElement>("button[aria-label='Close search']")!.click());
    await flushAsync();
    expect(searchField(container)).toBeNull();
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
