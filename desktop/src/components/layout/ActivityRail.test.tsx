// @vitest-environment jsdom
import type { ComponentProps } from "react";
import type { StoredThread } from "../../integrations/storage/threadStore";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { describe, expect, it, vi } from "vitest";
import { flushAsync } from "../../test/renderHook";
import { ActivityRail } from "./ActivityRail";

vi.mock("../../integrations/skills/skillsClient", () => ({ listInstalledSkills: async () => [] }));
vi.mock("../../integrations/agent/agentStateCache", () => ({ useCachedAgentState: () => undefined }));
vi.mock("../../lib/useIsFullscreen", () => ({ useIsFullscreen: () => false }));
vi.mock("./hooks/usePendingApprovalCounts", () => ({ usePendingApprovalCounts: () => new Map() }));
vi.mock("../../lib/useFloatingScrollbar", () => ({
  useFloatingScrollbar: () => ({ scrollRef: { current: null }, scrollbar: { height: 0, top: 0, visible: false }, handleScroll: () => {}, handleThumbPointerDown: () => {} }),
}));

function thread(id: string, parentSessionId?: string): StoredThread {
  return { id, parentSessionId, agentSessionId: id, title: id, mode: "chat", workspaceId: id, status: "active", pinned: false, readonly: false, createdAt: 0, updatedAt: 0 };
}

function workspaceThread(id: string, workspaceId: string): StoredThread {
  return { ...thread(id), mode: "workspace", workspaceId };
}

function props(threads: StoredThread[]): ComponentProps<typeof ActivityRail> {
  return {
    active: "chat",
    expanded: true,
    activeThreadId: "root",
    threads,
    threadRunStatuses: {},
    threadStreamingStatuses: {},
    unreadThreadIds: new Set(),
    workspaces: [],
    skillIntroDismissed: true,
    onDismissSkillIntro: vi.fn(),
    onChange: vi.fn(),
    onBatchDeleteThreads: vi.fn(),
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
    onToggleExpanded: vi.fn(),
  };
}

describe("activity rail conversation hierarchy", () => {
  it("restores collapsed workspace groups persisted by the previous session", async () => {
    localStorage.setItem("future.collapsedWorkspaces", JSON.stringify(["ws-a"]));
    const container = document.createElement("div");
    document.body.appendChild(container);
    const root = createRoot(container);
    const p: ComponentProps<typeof ActivityRail> = {
      ...props([workspaceThread("ws-thread", "ws-a")]),
      workspaces: [{ id: "ws-a", name: "Alpha", kind: "user", path: "/tmp/alpha", cleanupStatus: "active", createdAt: 0, updatedAt: 0 }],
    };
    act(() => root.render(<ActivityRail {...p} />));
    await flushAsync();
    const byTitle = (title: string) => container.querySelector<HTMLButtonElement>(`button[aria-label="${title}"]`);
    try {
      // Restored collapsed: the group chevron offers to expand and its chats stay hidden.
      expect(byTitle("Expand workspace")).not.toBeNull();
      expect(byTitle("ws-thread")).toBeNull();
      act(() => byTitle("Expand workspace")!.click());
      expect(byTitle("Collapse workspace")).not.toBeNull();
      expect(byTitle("ws-thread")).not.toBeNull();
      expect(JSON.parse(localStorage.getItem("future.collapsedWorkspaces")!)).toEqual([]);
    }
    finally {
      act(() => root.unmount());
      container.remove();
      localStorage.clear();
    }
  });

  it("expands independently from selection, starts collapsed, and retains state on status updates", async () => {
    const container = document.createElement("div");
    document.body.appendChild(container);
    const root = createRoot(container);
    const p = props([thread("grandchild", "child"), thread("child", "root"), thread("root")]);
    act(() => root.render(<ActivityRail {...p} />));
    await flushAsync();
    const byTitle = (title: string) => container.querySelector<HTMLButtonElement>(`button[aria-label="${title}"]`);
    try {
      expect(byTitle("child")).toBeNull();
      act(() => byTitle("root")!.click());
      expect(p.onSelectThread).toHaveBeenCalledWith(p.threads[2]);
      expect(byTitle("child")).toBeNull();
      const expand = byTitle("root")!.parentElement!.querySelector<HTMLButtonElement>("button[aria-expanded=false]")!;
      act(() => expand.click());
      expect(byTitle("child")).not.toBeNull();
      expect(byTitle("grandchild")).toBeNull();
      expect(p.onSelectThread).toHaveBeenCalledTimes(1);
      act(() => root.render(<ActivityRail {...p} threadStreamingStatuses={{ child: true }} />));
      expect(byTitle("child")).not.toBeNull();
      const childRow = byTitle("child")!.parentElement!;
      act(() => childRow.querySelector<HTMLButtonElement>("button[aria-expanded=false]")!.click());
      expect(byTitle("grandchild")).not.toBeNull();
      const rootRow = byTitle("root")!.parentElement!;
      act(() => rootRow.querySelector<HTMLButtonElement>("button[aria-expanded=true]")!.click());
      expect(byTitle("child")).toBeNull();
      expect(byTitle("grandchild")).toBeNull();
    }
    finally {
      act(() => root.unmount());
      container.remove();
    }
  });

  it("keeps the selection toolbar outside the shared workspace/chat viewport", async () => {
    const container = document.createElement("div");
    document.body.appendChild(container);
    const root = createRoot(container);
    act(() => root.render(<ActivityRail {...props([thread("root")])} />));
    await flushAsync();
    try {
      const menuTrigger = container.querySelector<HTMLButtonElement>("button[aria-label=\"Chat section actions\"]")!;
      act(() => menuTrigger.click());
      act(() => container.querySelector<HTMLButtonElement>("[role=\"menuitem\"]")!.click());

      const viewport = container.querySelector("[data-activity-rail-scroll=\"true\"]")!;
      const toolbar = container.querySelector("[data-activity-rail-selection-toolbar=\"true\"]")!;
      expect(viewport.contains(toolbar)).toBe(false);
      expect(viewport.parentElement?.parentElement?.contains(toolbar)).toBe(true);
      expect(viewport.querySelector("[data-activity-rail-content=\"true\"]")).not.toBeNull();
      expect(toolbar.classList.contains("-mb-2")).toBe(false);
      expect(toolbar.querySelectorAll(".activity-rail-selection-action")).toHaveLength(2);
      expect(toolbar.querySelectorAll(".activity-rail-selection-action-label")).toHaveLength(2);
      expect(toolbar.querySelector<HTMLButtonElement>("button[aria-label=\"Delete\"]")?.title).toBe("Delete");
      expect(toolbar.querySelector<HTMLButtonElement>("button[aria-label=\"Cancel\"]")?.title).toBe("Cancel");
    }
    finally {
      act(() => root.unmount());
      container.remove();
    }
  });
});
