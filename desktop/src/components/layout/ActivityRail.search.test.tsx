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

function thread(id: string, title: string, mode: "chat" | "workspace" = "chat", workspaceId = "ws"): ReturnType<typeof threadBuilder> {
  return threadBuilder(id, title, mode, workspaceId);
}

function threadBuilder(id: string, title: string, mode: "chat" | "workspace" = "chat", workspaceId = "ws") {
  return {
    id,
    agentSessionId: id,
    title,
    mode,
    workspaceId,
    status: "active" as const,
    pinned: false,
    readonly: false,
    createdAt: 0,
    updatedAt: 0,
  };
}

function props(threads: ReturnType<typeof thread>[] = [], workspaces: StoredWorkspace[] = []): ComponentProps<typeof ActivityRail> {
  return {
    active: "chat",
    expanded: true,
    activeThreadId: null,
    threads,
    threadRunStatuses: {},
    threadStreamingStatuses: {},
    unreadThreadIds: new Set(),
    workspaces,
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

describe("activity rail conversation search", () => {
  beforeEach(() => {
    localStorage.clear();
  });

  it("filters local and remote conversations by title", async () => {
    const p = props(
      [thread("a", "Alpha chat"), thread("b", "Beta chat")],
      [],
    );
    const { container } = await render(p);
    act(() => container.querySelector<HTMLButtonElement>("button[aria-label='Search conversations']")!.click());
    await flushAsync();
    const input = container.querySelector<HTMLInputElement>("input[aria-label='Search conversations']")!;
    await act(async () => {
      const descriptor = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value");
      descriptor?.set?.call(input, "Alpha");
      input.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await flushAsync();
    const list = container.querySelector("[data-activity-rail-content]")!;
    expect(list.textContent).toContain("Alpha chat");
    expect(list.textContent).not.toContain("Beta chat");
  });

  it("filters merged local+remote rows together", async () => {
    const p = {
      ...props([thread("local", "local matching")], []),
      remotePeers: [{
        agentAvailable: true,
        bridgeInstanceId: "bridge_1",
        connected: true,
        desktopId: "desktop_a",
        error: null,
        features: [] as string[],
        icon: "laptop",
        name: "Studio iMac",
        pairId: "pair_1",
      }],
      remoteCatalogs: [{
        desktopId: "desktop_a",
        sessions: [{
          sessionId: "remote_1",
          title: "remote matching",
          mode: "chat" as const,
          lastMessageAt: 1,
        }],
      }],
      onChangeDeviceFilter: vi.fn(),
      onManageDesktops: vi.fn(),
    };
    const { container } = await render(p);
    act(() => container.querySelector<HTMLButtonElement>("button[aria-label='Search conversations']")!.click());
    await flushAsync();
    const input = container.querySelector<HTMLInputElement>("input[aria-label='Search conversations']")!;
    await act(async () => {
      const descriptor = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value");
      descriptor?.set?.call(input, "matching");
      input.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await flushAsync();
    const list = container.querySelector("[data-activity-rail-content]")!;
    expect(list.textContent).toContain("local matching");
    expect(list.textContent).toContain("remote matching");
  });

  it("shows a 'no matches' hint when the filter excludes everything", async () => {
    const p = props([thread("a", "Alpha chat")], []);
    const { container } = await render(p);
    act(() => container.querySelector<HTMLButtonElement>("button[aria-label='Search conversations']")!.click());
    await flushAsync();
    const input = container.querySelector<HTMLInputElement>("input[aria-label='Search conversations']")!;
    await act(async () => {
      const descriptor = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value");
      descriptor?.set?.call(input, "zzz");
      input.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await flushAsync();
    const list = container.querySelector("[data-activity-rail-content]")!;
    expect(list.textContent).toContain("No matching conversations");
  });

  it("clears search when Escape is pressed", async () => {
    const p = props([thread("a", "Alpha chat")], []);
    const { container } = await render(p);
    act(() => container.querySelector<HTMLButtonElement>("button[aria-label='Search conversations']")!.click());
    await flushAsync();
    const input = container.querySelector<HTMLInputElement>("input[aria-label='Search conversations']")!;
    await act(async () => {
      const descriptor = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value");
      descriptor?.set?.call(input, "Alpha");
      input.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await flushAsync();
    await act(async () => {
      input.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    });
    await flushAsync();
    expect(container.querySelector("input[aria-label='Search conversations']")).toBeNull();
  });

  it("coexists with device filter and selection mode", async () => {
    const p = {
      ...props([thread("a", "Alpha chat"), thread("b", "Beta chat")], []),
      remotePeers: [{
        agentAvailable: true,
        bridgeInstanceId: "bridge_1",
        connected: true,
        desktopId: "desktop_a",
        error: null,
        features: [] as string[],
        icon: "laptop",
        name: "Studio iMac",
        pairId: "pair_1",
      }],
      remoteCatalogs: [{
        desktopId: "desktop_a",
        sessions: [{
          sessionId: "remote_1",
          title: "remote Alpha",
          mode: "chat" as const,
          lastMessageAt: 1,
        }],
      }],
      onChangeDeviceFilter: vi.fn(),
      onManageDesktops: vi.fn(),
    };
    const { container } = await render(p);
    act(() => container.querySelector<HTMLButtonElement>("button[aria-label='Search conversations']")!.click());
    await flushAsync();
    const input = container.querySelector<HTMLInputElement>("input[aria-label='Search conversations']")!;
    await act(async () => {
      const descriptor = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value");
      descriptor?.set?.call(input, "Alpha");
      input.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await flushAsync();
    const list = container.querySelector("[data-activity-rail-content]")!;
    expect(list.textContent).toContain("Alpha chat");
    expect(list.textContent).toContain("remote Alpha");
    expect(list.textContent).not.toContain("Beta chat");
  });
});
