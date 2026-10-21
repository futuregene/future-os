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

/** A conversation that lives in a workspace, for the grouping tests below. */
function workspaceThread(id: string, workspaceId: string, title: string): ReturnType<typeof thread> {
  return thread(id, title, "workspace", workspaceId);
}

function storedWorkspace(id: string, name: string): StoredWorkspace {
  return {
    id,
    name,
    kind: "user",
    path: `/tmp/${id}`,
    cleanupStatus: "active",
    createdAt: 0,
    updatedAt: 0,
  };
}

function props(threads: ReturnType<typeof thread>[] = [], workspaces: StoredWorkspace[] = []): ComponentProps<typeof ActivityRail> {
  return {
    active: "conversations",
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
    view: "workspace",
    onSelectView: vi.fn(),
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
        stopped: false,
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

  /**
   * Escape clears the query and leaves the field in place.
   *
   * The field is always on screen now, so there is no "open" state for Escape
   * to close: what it can do is take the filter away and hand the keyboard back
   * to the list. It answers only while the field has focus, which is the next
   * test's point.
   */
  it("clears the query on Escape and keeps the field", async () => {
    const p = { ...props([thread("a", "Alpha chat"), thread("b", "Beta chat")], []), view: "chat" as const };
    const { container } = await render(p);
    const input = container.querySelector<HTMLInputElement>("input[aria-label='Search conversations']")!;
    await act(async () => {
      const descriptor = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value");
      descriptor?.set?.call(input, "Alpha");
      input.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await flushAsync();
    const list = container.querySelector("[data-activity-rail-content]")!;
    expect(list.textContent).not.toContain("Beta chat");

    act(() => input.focus());
    await act(async () => {
      input.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    });
    await flushAsync();

    // Still there (it is not a mode), empty, and the whole list is back.
    const field = container.querySelector<HTMLInputElement>("input[aria-label='Search conversations']");
    expect(field).not.toBeNull();
    expect(field!.value).toBe("");
    expect(list.textContent).toContain("Beta chat");
  });

  /**
   * Escape pressed anywhere else must not wipe the filter.
   *
   * The field is always mounted, so its Escape listener is live while the reader
   * is reading the transcript. A bare window listener would empty a search the
   * reader is not looking at, from a keystroke meant for something else.
   */
  it("ignores Escape while the field does not have focus", async () => {
    const p = props([thread("a", "Alpha chat"), thread("b", "Beta chat")], []);
    const { container } = await render(p);
    const input = container.querySelector<HTMLInputElement>("input[aria-label='Search conversations']")!;
    await act(async () => {
      const descriptor = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value");
      descriptor?.set?.call(input, "Alpha");
      input.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await flushAsync();

    act(() => input.blur());
    act(() => {
      window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    });
    await flushAsync();

    expect(container.querySelector<HTMLInputElement>("input[aria-label='Search conversations']")!.value).toBe("Alpha");
    expect(container.querySelector("[data-activity-rail-content]")!.textContent).not.toContain("Beta chat");
  });

  it("coexists with device filter and selection mode", async () => {
    const p = {
      ...props([thread("a", "Alpha chat"), thread("b", "Beta chat")], []),
      remotePeers: [{
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

  it("empties the filter from the clear button, which only exists while there is a query", async () => {
    const p = { ...props([thread("a", "Alpha chat"), thread("b", "Beta chat")], []), view: "chat" as const };
    const { container } = await render(p);

    // Nothing typed: no clear affordance at all.
    expect(container.querySelector("button[aria-label='Clear search']")).toBeNull();

    const input = container.querySelector<HTMLInputElement>("input[aria-label='Search conversations']")!;
    await act(async () => {
      const descriptor = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value");
      descriptor?.set?.call(input, "Alpha");
      input.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await flushAsync();
    const list = container.querySelector("[data-activity-rail-content]")!;
    expect(list.textContent).not.toContain("Beta chat");

    act(() => container.querySelector<HTMLButtonElement>("button[aria-label='Clear search']")!.click());
    await flushAsync();

    // The filter is gone (both titles are back), the field is empty again, and
    // the clear button went with the query it belonged to.
    expect(container.querySelector<HTMLInputElement>("input[aria-label='Search conversations']")!.value).toBe("");
    expect(list.textContent).toContain("Alpha chat");
    expect(list.textContent).toContain("Beta chat");
    expect(container.querySelector("button[aria-label='Clear search']")).toBeNull();
  });

  /**
   * A workspace's own name answers the search — for the group, and for the rows
   * inside it.
   *
   * The field says "search conversations", but the rail draws a workspace's
   * conversations under that workspace's name, so a reader who types the name is
   * asking for what is on screen under it. Two rules do the work: a group whose
   * name matches is kept, and a thread is matched by its workspace's name as well
   * as its own title. Getting the second wrong hides exactly the row the reader
   * just saw grouped under the name they typed.
   */
  it("matches a workspace's name for the group and for its rows", async () => {
    const p = props(
      [
        workspaceThread("plain", "ws_charts", "an unremarkable title"),
        workspaceThread("other", "ws_other", "unrelated"),
      ],
      [storedWorkspace("ws_charts", "Charts workspace"), storedWorkspace("ws_other", "Other")],
    );
    const { container } = await render(p);
    const input = container.querySelector<HTMLInputElement>("input[aria-label='Search conversations']")!;
    await act(async () => {
      const descriptor = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value");
      descriptor?.set?.call(input, "charts");
      input.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await flushAsync();

    const list = container.querySelector("[data-activity-rail-content]")!;
    // The heading the name belongs to, and the row under it.
    expect(list.textContent).toContain("Charts workspace");
    expect(list.textContent).toContain("an unremarkable title");
    // The other workspace is gone, name and row alike.
    expect(list.textContent).not.toContain("Other");
    expect(list.textContent).not.toContain("unrelated");
  });

  /** A remote row carries its workspace's name too, and it answers the same way. */
  it("matches a remote row by its workspace's name", async () => {
    const p = {
      ...props([], []),
      remotePeers: [{
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
      }],
      remoteCatalogs: [{
        desktopId: "desktop_a",
        sessions: [{
          sessionId: "remote_1",
          title: "a title that says nothing",
          mode: "workspace" as const,
          workspaceId: "ws_remote",
          workspaceName: "Charts workspace",
          lastMessageAt: 1,
        }],
      }],
      onChangeDeviceFilter: vi.fn(),
      onManageDesktops: vi.fn(),
    };
    const { container } = await render(p);
    const input = container.querySelector<HTMLInputElement>("input[aria-label='Search conversations']")!;
    await act(async () => {
      const descriptor = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value");
      descriptor?.set?.call(input, "charts");
      input.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await flushAsync();

    const list = container.querySelector("[data-activity-rail-content]")!;
    expect(list.textContent).toContain("a title that says nothing");
  });

  /**
   * A workspace row is a result, so the rail must not also say there are none.
   *
   * A list of hits is assembled from every row the rail holds, not from whichever
   * half the current tab draws: reading one half tells a reader their
   * conversation does not exist while it is drawn a few rows down.
   */
  it("counts a matching workspace row as a result", async () => {
    const p = props([workspaceThread("ws_row", "ws_a", "the needle")], [storedWorkspace("ws_a", "Alpha")]);
    const { container } = await render(p);
    const input = container.querySelector<HTMLInputElement>("input[aria-label='Search conversations']")!;
    await act(async () => {
      const descriptor = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value");
      descriptor?.set?.call(input, "needle");
      input.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await flushAsync();

    const list = container.querySelector("[data-activity-rail-content]")!;
    expect(list.textContent).toContain("the needle");
    // The list's own no-matches banner is the first thing it draws; it must not
    // be there when a workspace group answered the search.
    expect(list.firstElementChild?.getAttribute("data-testid")).not.toBe("no-matches");
  });
  /**
   * The hard requirement: a hit in a workspace is never lost.
   *
   * Search spans the tabs, because a hit the reader cannot see is a hit they will
   * report as missing. This is the case that goes wrong first — the tabs are two
   * lists, so a search assembled from "the current tab's rows" answers "no
   * matches" while the conversation sits one tab away.
   */
  it("finds a workspace conversation while the chats list is the one showing", async () => {
    const p = {
      ...props([workspaceThread("ws_row", "ws_a", "the needle")], [storedWorkspace("ws_a", "Alpha")]),
      view: "chat" as const,
    };
    const { container } = await render(p);
    await act(async () => {
      const descriptor = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value");
      descriptor?.set?.call(container.querySelector<HTMLInputElement>("input[aria-label='Search conversations']")!, "needle");
      container.querySelector<HTMLInputElement>("input[aria-label='Search conversations']")!
        .dispatchEvent(new Event("input", { bubbles: true }));
    });
    await flushAsync();

    const list = container.querySelector("[data-activity-rail-content]")!;
    // The row, and where it lives — a flat result list otherwise leaves the
    // reader to open it to find out.
    expect(list.textContent).toContain("the needle");
    expect(list.textContent).toContain("Alpha");
    // And the rail says that the hit is not in the list that is current, so the
    // results below it are not read as "from here".
    const elsewhere = container.querySelector("[data-testid='search-elsewhere']");
    expect(elsewhere).not.toBeNull();
    expect(elsewhere!.textContent).toContain("Chats");
  });

  it("does not say 'elsewhere' when this tab answered the query itself", async () => {
    const p = { ...props([thread("a", "Alpha chat")], []), view: "chat" as const };
    const { container } = await render(p);
    await act(async () => {
      const descriptor = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value");
      descriptor?.set?.call(container.querySelector<HTMLInputElement>("input[aria-label='Search conversations']")!, "Alpha");
      container.querySelector<HTMLInputElement>("input[aria-label='Search conversations']")!
        .dispatchEvent(new Event("input", { bubbles: true }));
    });
    await flushAsync();

    expect(container.querySelector("[data-testid='search-elsewhere']")).toBeNull();
    expect(container.querySelector("[data-testid='no-matches']")).toBeNull();
    expect(container.querySelector("[data-testid='search-rows']")!.textContent).toContain("Alpha chat");
  });

  /**
   * Three empty answers, three sentences.
   *
   * They say different things, and reusing one makes a reader unable to tell
   * "this list is empty" from "your search matched nothing" from "your search
   * matched, in another list". Each is asserted by text, so a copy-paste that
   * collapses two of them fails here.
   */
  it("says a different thing for an empty list, no hits, and hits elsewhere", async () => {
    // (a) The list is empty and nothing is being searched for.
    const emptyChats = await render({ ...props([], []), view: "chat" as const });
    expect(emptyChats.container.querySelector("[data-testid='empty-chat-tab']")!.textContent).toBe("No chats");
    expect(emptyChats.container.querySelector("[data-testid='no-matches']")).toBeNull();
    emptyChats.root.unmount();

    const emptyWorkspaces = await render({ ...props([], []), view: "workspace" as const });
    expect(emptyWorkspaces.container.querySelector("[data-testid='empty-workspace-tab']")!.textContent).toBe("No workspaces yet");
    emptyWorkspaces.root.unmount();

    // (b) The search matched nothing anywhere.
    const none = await render({ ...props([thread("a", "Alpha chat")], []), view: "chat" as const });
    await act(async () => {
      const descriptor = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value");
      descriptor?.set?.call(none.container.querySelector<HTMLInputElement>("input[aria-label='Search conversations']")!, "zzz");
      none.container.querySelector<HTMLInputElement>("input[aria-label='Search conversations']")!
        .dispatchEvent(new Event("input", { bubbles: true }));
    });
    await flushAsync();
    const noMatches = none.container.querySelector("[data-testid='no-matches']")!;
    expect(noMatches.textContent).toBe("No matching conversations");
    expect(none.container.querySelector("[data-testid='search-elsewhere']")).toBeNull();
    none.root.unmount();

    // (c) It matched, in the other list — a different sentence again.
    const elsewhere = await render({
      ...props([workspaceThread("ws_row", "ws_a", "the needle")], [storedWorkspace("ws_a", "Alpha")]),
      view: "chat" as const,
    });
    await act(async () => {
      const descriptor = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value");
      descriptor?.set?.call(elsewhere.container.querySelector<HTMLInputElement>("input[aria-label='Search conversations']")!, "needle");
      elsewhere.container.querySelector<HTMLInputElement>("input[aria-label='Search conversations']")!
        .dispatchEvent(new Event("input", { bubbles: true }));
    });
    await flushAsync();
    const banner = elsewhere.container.querySelector("[data-testid='search-elsewhere']")!;
    expect(banner.textContent).not.toBe(noMatches.textContent);
    expect(banner.textContent).not.toBe("No chats");
    elsewhere.root.unmount();
  });
});
