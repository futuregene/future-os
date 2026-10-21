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
 * The rail's two bands: which list, then that list's own controls.
 *
 * These are *structure* assertions, not behaviour ones: the complaint they
 * answer is about where things are drawn, and a behaviour test that reads the
 * list would stay green with the search field back on a row of its own. So each
 * reads container relationships — which band an element is inside, and where in
 * it — which is the only shape that goes red when the layout is put back the way
 * it was.
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

const searchField = (root: HTMLElement) => root.querySelector<HTMLInputElement>("input[aria-label='Search conversations']");
/** The recent list's menu trigger — the "..." that is not a section heading's. */
const recentMenu = (root: HTMLElement) => root.querySelector<HTMLButtonElement>("button[aria-label='Recent list actions']");

describe("the rail's bands", () => {
  beforeEach(() => {
    localStorage.clear();
    document.body.innerHTML = "";
  });

  it("keeps the switch's band to the switch alone", async () => {
    const { container } = await render(props());
    // Read in the recent view, where the list's own menu exists at all: in the
    // tree view there is no such menu to misplace, so a default-view assertion
    // would pass on a layout that has it in the wrong band.
    act(() => container.querySelector<HTMLButtonElement>("[data-testid='rail-view-recent']")!.click());
    await flushAsync();

    const viewBand = band(container, "rail-view-band")!;
    expect(viewBand.querySelector("[data-testid='rail-view-switch']")).not.toBeNull();
    // Nothing that belongs to the list: no search field, no list menu. The menu
    // used to sit here, one pixel from the switch it does not act on.
    expect(viewBand.querySelector("input")).toBeNull();
    expect(recentMenu(viewBand)).toBeNull();
    expect(viewBand.querySelector("button[aria-haspopup='menu']")).toBeNull();
    // And the menu does exist — otherwise the assertions above would be about a
    // control that is not on screen at all.
    expect(recentMenu(container)).not.toBeNull();
  });

  it("keeps the search field on screen, in the list's own band, with no interaction", async () => {
    const { container } = await render(props());
    const tools = band(container, "rail-list-tools")!;
    const field = searchField(tools);
    expect(field).not.toBeNull();
    // Drain the click-into-a-mode path: the field used to appear only after a
    // glyph was clicked. If it is here with nothing done, that path is gone.
    expect(field!.value).toBe("");
    expect(band(container, "rail-view-band")!.contains(field)).toBe(false);
  });

  it("shares one row between the search field and the recent list's menu", async () => {
    const { container } = await render(props());
    act(() => container.querySelector<HTMLButtonElement>("[data-testid='rail-view-recent']")!.click());
    await flushAsync();

    const tools = band(container, "rail-list-tools")!;
    const trigger = recentMenu(tools);
    expect(trigger).not.toBeNull();
    // Same band as the field: that is the whole point of the change — one row
    // holding the list's actions, instead of a row of their own for each. A
    // sibling band would satisfy "in a band" and fail this.
    expect(tools.contains(searchField(tools))).toBe(true);
    // And at its right end.
    expect(tools.lastElementChild!.contains(trigger)).toBe(true);
  });

  it("orders the band: machine picker, search, then the list's menu", async () => {
    const p = props();
    p.remotePeers = [peer];
    p.onChangeDeviceFilter = vi.fn();
    p.onManageDesktops = vi.fn();
    const { container } = await render(p);
    act(() => container.querySelector<HTMLButtonElement>("[data-testid='rail-view-recent']")!.click());
    await flushAsync();

    const tools = band(container, "rail-list-tools")!;
    const children = Array.from(tools.children);
    const pickerIndex = children.findIndex(child => child.textContent?.includes("All devices"));
    const searchIndex = children.findIndex(child => child.contains(searchField(tools)));
    const menuIndex = children.findIndex(child => child.contains(recentMenu(tools)));
    expect(pickerIndex).toBe(0);
    expect(searchIndex).toBe(1);
    expect(menuIndex).toBe(2);
  });

  it("drops the list's band while a batch is being selected", async () => {
    const { container } = await render(props());
    act(() => container.querySelector<HTMLButtonElement>("[data-testid='rail-view-recent']")!.click());
    await flushAsync();
    act(() => recentMenu(container)!.click());
    await flushAsync();
    act(() => [...container.querySelectorAll<HTMLButtonElement>("button")]
      .find(button => button.textContent?.includes("Select chats"))!
      .click());
    await flushAsync();

    // The selection toolbar replaces the list's controls; the switch stays, since
    // which list the batch covers is still the question it answers.
    expect(band(container, "rail-list-tools")).toBeNull();
    expect(band(container, "rail-view-band")).not.toBeNull();
  });
});
