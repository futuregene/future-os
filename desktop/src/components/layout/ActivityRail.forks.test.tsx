// @vitest-environment jsdom
import type { ComponentProps } from "react";
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

function conversation(
  id: string,
  title: string,
  parentSessionId: string | null = null,
  pinned = false,
) {
  return {
    id,
    agentSessionId: id,
    parentSessionId,
    title,
    mode: "chat" as const,
    workspaceId: "ws",
    status: "active" as const,
    pinned,
    readonly: false,
    createdAt: 0,
    updatedAt: 0,
  };
}

function props(
  threads: ReturnType<typeof conversation>[],
  activeThreadId: string | null,
): ComponentProps<typeof ActivityRail> {
  return {
    active: "chat",
    expanded: true,
    activeThreadId,
    threads,
    threadRunStatuses: {},
    threadStreamingStatuses: {},
    unreadThreadIds: new Set(),
    workspaces: [],
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

async function render(p: ComponentProps<typeof ActivityRail>) {
  const container = document.createElement("div");
  document.body.appendChild(container);
  const root = createRoot(container);
  act(() => root.render(<ActivityRail {...p} />));
  await flushAsync();
  return { container, root };
}

describe("activity rail forks", () => {
  beforeEach(() => {
    localStorage.clear();
  });

  /**
   * A fork is drawn under the turn it came from, and the tree starts folded — so
   * creating one put its conversation in the center while its row stayed hidden.
   * Selecting a conversation opens what hides it.
   */
  it("shows the selected conversation even when it hangs under a folded parent", async () => {
    const { container } = await render(props([
      conversation("parent", "Original conversation"),
      conversation("fork", "Forked conversation", "parent"),
    ], "fork"));

    const list = container.querySelector("[data-activity-rail-content]")!;
    expect(list.textContent).toContain("Original conversation");
    expect(list.textContent).toContain("Forked conversation");
  });

  it("leaves an unrelated fold alone", async () => {
    const { container } = await render(props([
      conversation("other", "Folded conversation"),
      conversation("other-fork", "Hidden fork", "other"),
      conversation("plain", "Selected conversation"),
    ], "plain"));

    const list = container.querySelector("[data-activity-rail-content]")!;
    expect(list.textContent).toContain("Selected conversation");
    expect(list.textContent).not.toContain("Hidden fork");
  });

  it("opens the whole chain for a fork of a fork", async () => {
    const { container } = await render(props([
      conversation("root", "Root conversation"),
      conversation("middle", "Middle fork", "root"),
      conversation("leaf", "Leaf fork", "middle"),
    ], "leaf"));

    const list = container.querySelector("[data-activity-rail-content]")!;
    expect(list.textContent).toContain("Root conversation");
    expect(list.textContent).toContain("Middle fork");
    expect(list.textContent).toContain("Leaf fork");
  });
});
