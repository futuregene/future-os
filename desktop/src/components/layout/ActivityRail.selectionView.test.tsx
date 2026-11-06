// @vitest-environment jsdom
import type { ComponentProps } from "react";
import type { StoredThread } from "../../integrations/storage/threadStore";
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
// The archived stream reads the store through Tauri commands; this file is
// about the toolbar, so the shelf is mocked to an empty one.
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
    threads: [],
    threadsRemaining: 0,
    workspaceNextCursor: {},
    workspaceRemaining: {},
    workspaceThreads: {},
    workspaces: [],
  }),
}));

const now = Date.UTC(2026, 9, 21, 12, 0, 0);

function thread(id: string, workspaceId = "ws"): StoredThread {
  return {
    id,
    agentSessionId: id,
    title: id,
    mode: "workspace",
    workspaceId,
    status: "active",
    pinned: false,
    readonly: false,
    createdAt: now,
    updatedAt: now,
    lastMessageAt: now,
  };
}

function props(threads: StoredThread[]): ComponentProps<typeof ActivityRail> {
  return {
    active: "workspace",
    activeThreadId: null,
    expanded: true,
    onBatchDeleteThreads: vi.fn(),
    onDeleteRemoteConversations: vi.fn(),
    onRemoteConversationDeleted: vi.fn(),
    onChange: vi.fn(),
    onDeleteThread: vi.fn(),
    onDeleteWorkspace: vi.fn(),
    onDismissSkillIntro: vi.fn(),
    onNewChat: vi.fn(),
    onNewWorkspace: vi.fn(),
    onOpenModels: vi.fn(),
    onRenameThread: vi.fn(),
    onRenameWorkspace: vi.fn(),
    onRestoreThread: vi.fn(),
    onSelectThread: vi.fn(),
    onSelectWorkspace: vi.fn(),
    onToggleExpanded: vi.fn(),
    onTogglePinThread: vi.fn(),
    onTogglePinWorkspace: vi.fn(),
    skillIntroDismissed: true,
    threadRunStatuses: {},
    threadStreamingStatuses: {},
    threads,
    unreadThreadIds: new Set<string>(),
    workspaces: [{
      id: "ws",
      name: "case",
      kind: "user" as const,
      path: "/tmp/ws",
      cleanupStatus: "active" as const,
      createdAt: 0,
      updatedAt: 0,
    }],
  };
}

/** The batch toolbar, if one is on screen. */
function toolbar(container: HTMLElement) {
  return container.querySelector("[data-activity-rail-selection-toolbar=\"true\"]");
}

/** Open the workspace heading's menu and choose "Select chats". */
function enterBatchFromWorkspace(container: HTMLElement) {
  const trigger = container.querySelector<HTMLButtonElement>("button[aria-label^=\"Workspace actions for \"]")!;
  act(() => trigger.click());
  const item = [...container.querySelectorAll<HTMLElement>("[role=\"menuitem\"]")]
    .find(element => element.textContent === "Select chats")!;
  act(() => item.click());
}

describe("a batch opened from a workspace heading", () => {
  beforeEach(() => {
    localStorage.clear();
    document.body.innerHTML = "";
  });

  /**
   * Switching the list closes the batch.
   *
   * A batch is "the rows under this heading", and a heading belongs to one list:
   * the tree's workspaces and chat section, or the archived shelves. The other
   * list has no such heading, so the batch it would act on is not on screen —
   * and the toolbar it keeps drawing has a count, a select-all and a Delete
   * over rows the reader is no longer looking at. Reported as: "in a workspace,
   * click 选择聊天, then switch between 最近 / 归档 — 多选 never goes away".
   */
  it.each([
    ["recent", () => "[data-testid='rail-view-recent']"],
    ["archive", () => "[data-testid='rail-archive-entry']"],
  ])("ends when the list becomes %s", async (_name, selector) => {
    const container = document.createElement("div");
    document.body.appendChild(container);
    const root = createRoot(container);
    act(() => root.render(<ActivityRail {...props([thread("a")])} />));
    await flushAsync();

    enterBatchFromWorkspace(container);
    await flushAsync();
    expect(toolbar(container)).not.toBeNull();

    act(() => container.querySelector<HTMLButtonElement>(selector())!.click());
    await flushAsync();
    expect(toolbar(container)).toBeNull();

    act(() => root.unmount());
    container.remove();
  });

  /** And it does not come back when the reader returns to the tree. */
  it("stays closed after switching back to the list it was opened in", async () => {
    const container = document.createElement("div");
    document.body.appendChild(container);
    const root = createRoot(container);
    act(() => root.render(<ActivityRail {...props([thread("a")])} />));
    await flushAsync();

    enterBatchFromWorkspace(container);
    await flushAsync();
    expect(toolbar(container)).not.toBeNull();

    act(() => container.querySelector<HTMLButtonElement>("[data-testid='rail-view-recent']")!.click());
    await flushAsync();
    act(() => container.querySelector<HTMLButtonElement>("[data-testid='rail-view-workspace']")!.click());
    await flushAsync();

    expect(toolbar(container)).toBeNull();
    act(() => root.unmount());
    container.remove();
  });
});
