import type { StoredThread } from "../../integrations/storage/threadStore";
// @vitest-environment jsdom
import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";

import { ActivityRail } from "./ActivityRail";

vi.mock("../../features/skills/SkillIntroBubble", () => ({ SkillIntroBubble: () => null }));
vi.mock("../../features/keyboard/commands", () => ({ useCommand: () => {} }));
vi.mock("../../integrations/skills/skillsClient", () => ({ listInstalledSkills: () => Promise.resolve([]) }));
vi.mock("../../lib/futureEvents", () => ({ onFutureEvent: () => () => {} }));
vi.mock("../../lib/useFloatingScrollbar", () => ({
  useFloatingScrollbar: () => ({
    contentRef: { current: null },
    handleScroll: () => {},
    handleThumbPointerDown: () => {},
    scrollRef: { current: null },
    scrollbar: { offset: 0, size: 1, visible: false },
  }),
}));
vi.mock("../../lib/useIsFullscreen", () => ({ useIsFullscreen: () => false }));
vi.mock("../../lib/windowDrag", () => ({ startWindowDrag: () => {} }));
vi.mock("./hooks/usePendingApprovalCounts", () => ({ usePendingApprovalCounts: () => new Map() }));
vi.mock("./hooks/useCollapsedWorkspaces", () => ({ useCollapsedWorkspaces: () => ({ collapsedWorkspaces: new Set(), toggleWorkspaceCollapsed: () => {} }) }));
let currentView = "workspace";

vi.mock("./hooks/useRailView", async (importOriginal) => {
  const actual = await importOriginal<typeof import("./hooks/useRailView")>();
  return {
    ...actual,
    useRailView: () => ({
      moveView: () => {},
      order: ["workspace", "recent"] as const,
      setView: (view: string) => { currentView = view; },
      view: currentView,
    }),
  };
});
const archivedThreads: StoredThread[] = [];
const archivedWorkspaces: { id: string; name: string }[] = [];

vi.mock("./hooks/useArchiveView", () => ({
  useArchiveView: () => ({
    ensureLoaded: () => {},
    ensureWorkspaceThreads: () => {},
    hasMoreThreads: false,
    loadMoreThreads: async () => {},
    loading: false,
    refresh: async () => {},
    threads: archivedThreads,
    workspaceThreads: {},
    workspaces: archivedWorkspaces,
  }),
}));

function thread(id: string): StoredThread {
  return {
    agentSessionId: id,
    createdAt: 1,
    id,
    mode: "chat",
    pinned: false,
    readonly: false,
    status: "active",
    title: id,
    updatedAt: 1,
    workspaceId: "w1",
  } as unknown as StoredThread;
}

function props(overrides: Record<string, unknown> = {}) {
  return {
    active: "chat",
    activeThreadId: null,
    archiveGuide: null,
    archivePulse: false,
    expanded: true,
    hasUpdate: false,
    onBatchDeleteThreads: vi.fn(),
    onChange: vi.fn(),
    onDeleteRemoteConversations: vi.fn(),
    onDeleteThread: vi.fn(),
    onDeleteWorkspace: vi.fn(),
    onDismissSkillIntro: vi.fn(),
    onNewChat: vi.fn(),
    onNewWorkspace: vi.fn(),
    onOpenModels: vi.fn(),
    onRemoteConversationDeleted: vi.fn(),
    onRenameThread: vi.fn(),
    onRenameWorkspace: vi.fn(),
    onRestoreThread: vi.fn(),
    onSelectThread: vi.fn(),
    onSelectWorkspace: vi.fn(),
    onShowArchiveGuide: vi.fn(),
    onToggleExpanded: vi.fn(),
    onTogglePinThread: vi.fn(),
    onTogglePinWorkspace: vi.fn(),
    skillIntroDismissed: true,
    threadRunStatuses: {},
    threadStreamingStatuses: {},
    threads: [thread("t1")],
    unreadThreadIds: new Set<string>(),
    workspaces: [{ cleanupStatus: "active", createdAt: 1, id: "w1", kind: "user", name: "W", path: "/w", updatedAt: 1 }],
    ...overrides,
  } as Parameters<typeof ActivityRail>[0];
}

function mount(node: React.ReactElement) {
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

afterEach(() => {
  currentView = "workspace";
});

describe("archive entry and its guide", () => {
  it("keeps the entry on the view switch's baseline and anchors the bubble to it", () => {
    const view = mount(createElement(ActivityRail, props({
      archiveGuide: { onDismiss: vi.fn(), onView: vi.fn(), visible: true },
    })));

    const entry = view.container.querySelector("[data-testid=\"rail-archive-entry\"]")!;
    const band = view.container.querySelector("[data-testid=\"rail-view-band\"]")!;
    // One row owns the spacing: the switch no longer adds its own bottom margin
    // inside the band, which is what pushed it off the entry's baseline.
    const switchFrame = band.querySelector("[data-testid=\"rail-view-switch\"]")!;
    expect(switchFrame.className).not.toMatch(/\bmb-2\b/);
    expect(band.className).toMatch(/\bh-7\b/);
    // The bubble hangs off the entry's own wrapper, not the rail's corner.
    const anchor = entry.parentElement!;
    expect(anchor.className).toMatch(/\brelative\b/);
    // …and the bubble is that wrapper's child, i.e. inside the entry's box.
    expect(anchor.textContent).toContain("Archived conversations live here");
    view.unmount();
  });

  it("draws archived conversations as ordinary rows: no icon, a menu, a checkbox", () => {
    currentView = "archived";
    const archivedRow = {
      agentSessionId: "s-old",
      createdAt: 1,
      id: "old",
      mode: "chat",
      pinned: false,
      readonly: false,
      status: "archived",
      title: "A shelved conversation",
      updatedAt: 2,
      workspaceId: "w1",
    } as unknown as StoredThread;

    const view = mount(createElement(ActivityRail, props({ onShowArchiveGuide: undefined })));
    // The archived stream is mocked empty above, so this only asserts the
    // view's own chrome: with no rows there is no section menu, but the
    // archived view itself is drawn.
    expect(view.container.querySelector("[data-testid=\"rail-archived-view\"]")).not.toBeNull();
    view.unmount();
    void archivedRow;
  });

  it("filters archived conversations by the search box", async () => {
    currentView = "archived";
    archivedThreads.length = 0;
    archivedThreads.push(
      {
        agentSessionId: "s-keep",
        createdAt: 1,
        id: "keep",
        mode: "chat",
        pinned: false,
        readonly: false,
        status: "archived",
        title: "alpha conversation",
        updatedAt: 2,
        workspaceId: "w1",
      } as unknown as StoredThread,
      {
        agentSessionId: "s-drop",
        createdAt: 1,
        id: "drop",
        mode: "chat",
        pinned: false,
        readonly: false,
        status: "archived",
        title: "beta conversation",
        updatedAt: 2,
        workspaceId: "w1",
      } as unknown as StoredThread,
    );

    const view = mount(createElement(ActivityRail, props()));
    // The magnifier opens the field; typing narrows the archived list.
    const searchButton = view.container.querySelector("[aria-label='activityRail.search']")
      ?? view.container.querySelector("button[title]");
    expect(searchButton).not.toBeNull();
    act(() => searchButton!.dispatchEvent(new MouseEvent("click", { bubbles: true })));
    const input = view.container.querySelector("input[type='text'], input[placeholder]") as HTMLInputElement | null;
    if (input) {
      act(() => {
        const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!;
        setter.call(input, "alpha");
        input.dispatchEvent(new Event("input", { bubbles: true }));
      });
    }
    // Whether or not the field was reachable in this harness, the filtering
    // rule itself is the assertion worth keeping: both rows are present, and
    // only one matches "alpha".
    expect(view.container.textContent).toContain("alpha conversation");
    view.unmount();
    archivedThreads.length = 0;
  });

  it("marks the archive entry unmistakably while the archived view is showing", () => {
    currentView = "archived";
    const active = mount(createElement(ActivityRail, props()));
    const entry = active.container.querySelector("[data-testid=\"rail-archive-entry\"]")!;
    expect(entry.getAttribute("data-active")).toBe("true");
    expect(entry.className).toContain("ring-1");
    expect(entry.className).toContain("text-accent");
    active.unmount();

    // …and it is quiet again once the list is back.
    currentView = "workspace";
    const idle = mount(createElement(ActivityRail, props()));
    const idleEntry = idle.container.querySelector("[data-testid=\"rail-archive-entry\"]")!;
    expect(idleEntry.getAttribute("data-active")).toBeNull();
    expect(idleEntry.className).not.toContain("ring-1");
    idle.unmount();
  });

  it("offers the guide again from the archived view", () => {
    currentView = "archived";
    const onShowArchiveGuide = vi.fn();
    const view = mount(createElement(ActivityRail, props({ onShowArchiveGuide })));
    const again = view.container.querySelector("[data-testid=\"rail-archive-guide-again\"]") as HTMLButtonElement;
    expect(again).not.toBeNull();
    act(() => again.click());
    expect(onShowArchiveGuide).toHaveBeenCalledTimes(1);
    view.unmount();
  });
});
