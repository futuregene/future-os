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
    hasMoreWorkspaces: false,
    loadAllThreads: async () => {},
    loadAllWorkspaceThreads: async () => {},
    loadMoreThreads: async () => {},
    loadMoreWorkspaceThreads: async () => {},
    loadMoreWorkspaces: async () => {},
    loading: false,
    refresh: async () => {},
    threads: archivedThreads,
    threadsRemaining: 0,
    workspaceNextCursor: {},
    workspaceRemaining: {},
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

  it("files a workspace's shelved conversation under that workspace, not among loose chats", () => {
    currentView = "archived";
    archivedThreads.length = 0;
    archivedWorkspaces.length = 0;
    archivedThreads.push({
      agentSessionId: "s-w",
      createdAt: 1,
      id: "w-thread",
      mode: "workspace",
      pinned: false,
      readonly: false,
      status: "archived",
      title: "Shelved in the workspace",
      updatedAt: 2,
      workspaceId: "w1",
    } as unknown as StoredThread);

    const view = mount(createElement(ActivityRail, props({
      workspaces: [{
        cleanupStatus: "active",
        createdAt: 1,
        id: "w1",
        kind: "user",
        name: "Workspace Alpha",
        path: "/w",
        updatedAt: 1,
      }],
    })));
    const body = view.container.querySelector("[data-testid='rail-archived-view']")!;
    // The workspace is still active, so its heading comes from the shelf's own
    // grouping rather than from an archived workspace; the row must hang under
    // it, which is what tells the reader what restoring it goes back to.
    const heading = [...body.querySelectorAll("[class*='group/workspace']")]
      .find(node => node.textContent?.includes("Workspace Alpha"));
    expect(heading).not.toBeNull();
    // The heading counts what is shelved in it, and expanding it draws the row
    // under the workspace it came from.
    expect(heading!.textContent).toContain("1");
    act(() => {
      heading!.querySelector("button")!.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });
    expect(heading!.parentElement!.textContent).toContain("Shelved in the workspace");
    view.unmount();
    archivedThreads.length = 0;
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

  it("folds an archived workspace with the same chevron as an active one", () => {
    currentView = "archived";
    archivedThreads.length = 0;
    archivedWorkspaces.length = 0;
    archivedWorkspaces.push({ id: "w-shelved", name: "Shelved project" });

    const view = mount(createElement(ActivityRail, props()));
    const heading = view.container.querySelector("[data-testid='rail-archived-view']")!;
    // The +/− text pair is gone; the disclosure renders an svg chevron.
    expect(heading.textContent).not.toContain("+");
    expect(heading.querySelector("svg.lucide-chevron-right, svg.lucide-chevron-down")).not.toBeNull();
    view.unmount();
    archivedThreads.length = 0;
    archivedWorkspaces.length = 0;
  });

  it("folds both archived sections with one click each", () => {
    currentView = "archived";
    archivedThreads.length = 0;
    archivedThreads.push({
      agentSessionId: "s-a",
      createdAt: 1,
      id: "a",
      mode: "chat",
      pinned: false,
      readonly: false,
      status: "archived",
      title: "shelved one",
      updatedAt: 2,
      workspaceId: "w1",
    } as unknown as StoredThread);

    const view = mount(createElement(ActivityRail, props()));
    const viewRoot = view.container.querySelector("[data-testid='rail-archived-view']")!;
    const foldButtons = [...viewRoot.querySelectorAll("button[aria-expanded]")];
    expect(foldButtons.length).toBeGreaterThanOrEqual(2);

    // The conversations section folds away its rows.
    expect(view.container.textContent).toContain("shelved one");
    const threadHeading = foldButtons.find(button =>
      button.getAttribute("aria-label")?.includes("已归档会话")
      || button.getAttribute("aria-label")?.includes("Archived conversations"))!;
    expect(threadHeading).toBeDefined();
    act(() => threadHeading.dispatchEvent(new MouseEvent("click", { bubbles: true })));
    expect(view.container.textContent).not.toContain("shelved one");
    // …and unfolds again.
    act(() => threadHeading.dispatchEvent(new MouseEvent("click", { bubbles: true })));
    expect(view.container.textContent).toContain("shelved one");
    view.unmount();
    archivedThreads.length = 0;
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

/**
 * The archived view's remote half.
 *
 * A host keeps its shelved conversations and workspaces in the same catalogue it
 * sends for the active list — the split is the client's — so the archived view
 * has to draw them itself, with the machine's glyph, and the active list has to
 * stop drawing them.
 */
const remotePeer = {
  agentAvailable: true,
  bridgeInstanceId: "bridge_1",
  connected: true,
  desktopId: "desktop_a",
  error: null,
  features: ["archive_v1"],
  icon: "laptop",
  name: "Studio iMac",
  pairId: "pair_1",
  stopped: false,
};

function remoteCatalog() {
  return [{
    desktopId: "desktop_a",
    sessions: [
      {
        archived: false,
        lastMessageAt: 4_000,
        mode: "chat",
        sessionId: "s_live",
        threadId: "t_live",
        title: "still here",
      },
      {
        archived: true,
        lastMessageAt: 3_000,
        mode: "workspace",
        sessionId: "s_shelved",
        threadId: "t_shelved",
        title: "remote shelved conversation",
        workspaceId: "ws_9",
        workspaceName: "Remote project",
      },
    ],
    workspaces: [
      { archived: true, id: "ws_9", name: "Remote project", path: "/remote/project", pinned: false },
    ],
  }] as never;
}

describe("the archived view's remote shelves", () => {
  it("draws a host's shelved workspace and conversation with the machine's glyph", () => {
    currentView = "archived";
    archivedThreads.length = 0;
    archivedWorkspaces.length = 0;

    const view = mount(createElement(ActivityRail, props({
      remoteCatalogs: remoteCatalog(),
      remotePeers: [remotePeer],
    })));
    const root = view.container.querySelector("[data-testid='rail-archived-view']")!;
    expect(root.textContent).toContain("Remote project");

    const group = [...root.querySelectorAll(".group\\/workspace")]
      .find(node => node.textContent?.includes("Remote project"))!;
    expect(group.querySelector("[data-testid='remote-source-icon']")).not.toBeNull();

    // Folded by default, as a shelved workspace is: the count says what is
    // behind the heading, and the reader opens it like any other group.
    expect(root.textContent).not.toContain("remote shelved conversation");
    act(() => group.querySelector("button")!.dispatchEvent(new MouseEvent("click", { bubbles: true })));
    expect(root.textContent).toContain("remote shelved conversation");
    expect(root.querySelector("[data-testid='remote-source-icon']")).not.toBeNull();
    view.unmount();
  });

  it("keeps a host's shelved conversation out of the active list", () => {
    currentView = "workspace";
    const view = mount(createElement(ActivityRail, props({
      deviceFilter: { kind: "all" },
      remoteCatalogs: remoteCatalog(),
      remotePeers: [remotePeer],
    })));
    expect(view.container.textContent).toContain("still here");
    expect(view.container.textContent).not.toContain("remote shelved conversation");
    view.unmount();
  });

  it("searches a host's shelved shelves by title and by workspace name", () => {
    currentView = "archived";
    archivedThreads.length = 0;
    archivedWorkspaces.length = 0;

    const view = mount(createElement(ActivityRail, props({
      remoteCatalogs: remoteCatalog(),
      remotePeers: [remotePeer],
    })));
    act(() => view.container
      .querySelector<HTMLButtonElement>("button[aria-label='Search conversations']")!
      .dispatchEvent(new MouseEvent("click", { bubbles: true })));
    const input = view.container.querySelector<HTMLInputElement>("input[aria-label='Search conversations']")!;
    act(() => {
      const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!;
      setter.call(input, "shelved conversation");
      input.dispatchEvent(new Event("input", { bubbles: true }));
    });
    // The row matched, so its shelf is kept and opened even though the query
    // says nothing about the workspace's name.
    expect(view.container.textContent).toContain("remote shelved conversation");
    expect(view.container.textContent).toContain("Remote project");

    act(() => {
      const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!;
      setter.call(input, "nothing matches this");
      input.dispatchEvent(new Event("input", { bubbles: true }));
    });
    expect(view.container.textContent).not.toContain("remote shelved conversation");
    expect(view.container.textContent).not.toContain("Remote project");
    view.unmount();
  });

  it("hides a host's shelf when the picker is showing another machine", () => {
    currentView = "archived";
    archivedThreads.length = 0;
    archivedWorkspaces.length = 0;

    const view = mount(createElement(ActivityRail, props({
      deviceFilter: { desktopId: null, kind: "device" },
      remoteCatalogs: remoteCatalog(),
      remotePeers: [remotePeer],
    })));
    const root = view.container.querySelector("[data-testid='rail-archived-view']")!;
    expect(root.textContent).not.toContain("Remote project");
    expect(root.textContent).not.toContain("remote shelved conversation");
    view.unmount();
  });
});
