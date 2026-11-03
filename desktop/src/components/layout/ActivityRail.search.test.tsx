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

  it("draws one × while a query is typed, not a clear and a close side by side", async () => {
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

    // One button, and it is the one that answers the state: clear.
    const dismiss = container.querySelectorAll("button[aria-label='Clear search'], button[aria-label='Close search']");
    expect(dismiss).toHaveLength(1);
    expect(dismiss[0]!.getAttribute("aria-label")).toBe("Clear search");

    act(() => (dismiss[0] as HTMLButtonElement).click());
    await flushAsync();
    const close = container.querySelector<HTMLButtonElement>("button[aria-label='Close search']");
    expect(close).not.toBeNull();
    act(() => close!.click());
    await flushAsync();
    expect(container.querySelector("input[aria-label='Search conversations']")).toBeNull();
  });

  it("empties the filter from the clear button, which only exists while there is a query", async () => {
    const p = props([thread("a", "Alpha chat"), thread("b", "Beta chat")], []);
    const { container } = await render(p);
    act(() => container.querySelector<HTMLButtonElement>("button[aria-label='Search conversations']")!.click());
    await flushAsync();

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
    act(() => container.querySelector<HTMLButtonElement>("button[aria-label='Search conversations']")!.click());
    await flushAsync();
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
    act(() => container.querySelector<HTMLButtonElement>("button[aria-label='Search conversations']")!.click());
    await flushAsync();
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

  it("narrows a host's workspace headings by the query, and drops the machine it is not showing", async () => {
    // The remote half of the tree is a second list of headings: a host's workspaces
    // that match the query, grouped under that host. Two rules meet here — the device
    // filter picks the machines, the query picks the headings — and a machine the
    // filter excludes must not contribute even when its workspaces match.
    const peer = (desktopId: string, name: string) => ({
      agentAvailable: true,
      bridgeInstanceId: `bridge_${desktopId}`,
      connected: true,
      error: null,
      features: [] as string[],
      icon: "laptop",
      name,
      pairId: `pair_${desktopId}`,
      stopped: false,
      desktopId,
    });
    const p = {
      ...props([], []),
      deviceFilter: { desktopId: "desktop_a", kind: "device" as const },
      remotePeers: [peer("desktop_a", "Studio iMac"), peer("desktop_b", "Attic PC")],
      remoteCatalogs: [
        {
          desktopId: "desktop_a",
          sessions: [],
          workspaces: [
            { id: "ws_a1", name: "Charts", path: "/charts", pinned: false },
            { id: "ws_a2", name: "Notes", path: "/notes", pinned: false },
          ],
        },
        { desktopId: "desktop_b", sessions: [], workspaces: [{ id: "ws_b1", name: "Charts archive", path: "/archive", pinned: false }] },
      ],
      onChangeDeviceFilter: vi.fn(),
      onManageDesktops: vi.fn(),
    };
    const { container } = await render(p);
    act(() => container.querySelector<HTMLButtonElement>("button[aria-label='Search conversations']")!.click());
    await flushAsync();
    const input = container.querySelector<HTMLInputElement>("input[aria-label='Search conversations']")!;
    await act(async () => {
      const descriptor = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value");
      descriptor?.set?.call(input, "char");
      input.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await flushAsync();

    const list = container.querySelector("[data-activity-rail-content]")!;
    // The filtered machine's matching heading is drawn …
    expect(list.textContent).toContain("Charts");
    // … its non-matching one is not …
    expect(list.textContent).not.toContain("Notes");
    // … and the machine the device filter excludes contributes nothing, even though
    // its workspace also matches.
    expect(list.textContent).not.toContain("Charts archive");
  });

  /**
   * A workspace row is a result, so the rail must not also say there are none.
   *
   * The pinned and chat sections are read first and can both be empty; the answer
   * then comes from the workspace groups. Reading only the first two tells a
   * reader their conversation does not exist while it is drawn under a heading
   * further down the list. (The chat *section's* own empty hint is a different
   * element and stays: that section really is empty.)
   */
  it("counts a matching workspace row as a result", async () => {
    const p = props([workspaceThread("ws_row", "ws_a", "the needle")], [storedWorkspace("ws_a", "Alpha")]);
    const { container } = await render(p);
    act(() => container.querySelector<HTMLButtonElement>("button[aria-label='Search conversations']")!.click());
    await flushAsync();
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
});
