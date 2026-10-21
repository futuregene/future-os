// @vitest-environment jsdom
import type { ComponentProps, ReactElement } from "react";
import type { StoredThread } from "../../integrations/storage/threadStore";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { resetAllCommandBindings } from "../../features/keyboard/shortcuts";
import { installShortcuts } from "../../test/installShortcuts";
import { ActivityRail } from "./ActivityRail";
import { RECENT_PAGE_SIZE } from "./recentOrder";

/**
 * The rail's second view: every conversation in one list, split by whether its
 * run is still in flight.
 *
 * The assertions read the *document order of the drawn rows*, because that is
 * what the view is: where a row sits relative to the others. "The title is
 * somewhere on the page" would pass for a list in any order at all.
 */

vi.mock("../../integrations/skills/skillsClient", () => ({ listInstalledSkills: async () => [] }));
vi.mock("../../integrations/agent/agentStateCache", () => ({ useCachedAgentState: () => undefined }));
vi.mock("../../lib/useIsFullscreen", () => ({ useIsFullscreen: () => false }));
vi.mock("./hooks/usePendingApprovalCounts", () => ({ usePendingApprovalCounts: () => new Map() }));
vi.mock("../../lib/useFloatingScrollbar", () => ({
  useFloatingScrollbar: () => ({ scrollRef: { current: null }, scrollbar: { height: 0, top: 0, visible: false }, handleScroll: () => {}, handleThumbPointerDown: () => {} }),
}));

const minute = 60_000;
const hour = 60 * minute;
const now = Date.UTC(2026, 9, 21, 12, 0, 0);

function thread(id: string, overrides: Partial<StoredThread> = {}): StoredThread {
  return {
    id,
    agentSessionId: id,
    title: id,
    mode: "chat",
    workspaceId: `ws-${id}`,
    status: "active",
    pinned: false,
    readonly: false,
    createdAt: now - 24 * hour,
    updatedAt: now,
    lastMessageAt: now,
    ...overrides,
  };
}

function mount(node: ReactElement) {
  const container = document.createElement("div");
  document.body.appendChild(container);
  const root = createRoot(container);
  act(() => root.render(node));
  return {
    container,
    rerender: (next: ReactElement) => act(() => root.render(next)),
    unmount: () => {
      act(() => root.unmount());
      container.remove();
    },
  };
}

function props(
  threads: StoredThread[],
  overrides: Partial<ComponentProps<typeof ActivityRail>> = {},
): ComponentProps<typeof ActivityRail> {
  return {
    active: "chat" as const,
    activeThreadId: null,
    expanded: true,
    futureSessionStatus: "authenticated",
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
    userEmail: "alice@example.com",
    workspaces: [],
    ...overrides,
  };
}

/**
 * The rows' own checkboxes, by label.
 *
 * The batch's toolbar has its own select-all box; these are the rows'.
 */
function rowCheckboxes(container: HTMLElement): string[] {
  return [...container.querySelectorAll("input[type=\"checkbox\"]")]
    .map(box => box.getAttribute("aria-label") ?? "")
    .filter(label => label.startsWith("Select "));
}

/** Switch the rail to the second view, the way a reader does. */
function showRecent(container: HTMLElement) {
  const trigger = container.querySelector<HTMLButtonElement>("[data-testid=\"rail-view-recent\"]")!;
  act(() => trigger.click());
}

/** A workspace, for the cases that need the tree half to have something to draw. */
function workspace(id: string, name: string) {
  return {
    id,
    name,
    kind: "user" as const,
    path: `/tmp/${id}`,
    cleanupStatus: "active" as const,
    createdAt: 0,
    updatedAt: 0,
  };
}

/** The row titles the list draws, in document order. */
function drawnTitles(container: HTMLElement): string[] {
  const list = container.querySelector("[data-testid=\"recent-rows\"]");
  if (!list)
    return [];
  // The title is the row's full-width click target, so this reads what the row
  // *is* rather than what its text happens to contain (the age is on the row
  // too, and would otherwise be read as part of the title).
  return [...list.querySelectorAll<HTMLElement>("[data-testid=\"recent-row\"]")]
    .map(row => row.querySelector("button")?.getAttribute("aria-label") ?? "");
}

/** What each caption says, in order, with the count it reports. */
function captions(container: HTMLElement): string[] {
  const list = container.querySelector("[data-testid=\"recent-rows\"]");
  if (!list)
    return [];
  return [...list.querySelectorAll<HTMLElement>("[data-recent-section]")].map((section) => {
    const kind = section.getAttribute("data-recent-section");
    const label = section.querySelector("[data-recent-count]")?.parentElement?.textContent?.replace(/\d+$/, "") ?? "";
    const count = section.querySelector("[data-recent-count]")?.getAttribute("data-recent-count");
    return `${kind}:${label.trim()}:${count}`;
  });
}

beforeEach(() => {
  localStorage.clear();
});
afterEach(() => {
  localStorage.clear();
});

describe("the rail's view switch", () => {
  it("opens on the tree, and the switch says which list is showing", () => {
    const view = mount(<ActivityRail {...props([thread("a")])} />);
    const tree = view.container.querySelector("[data-testid=\"rail-view-workspace\"]")!;
    const recent = view.container.querySelector("[data-testid=\"rail-view-recent\"]")!;
    expect(tree.getAttribute("aria-pressed")).toBe("true");
    expect(recent.getAttribute("aria-pressed")).toBe("false");
    // No recent list yet: the tree is what is drawn.
    expect(view.container.querySelector("[data-testid=\"recent-rows\"]")).toBeNull();
    view.unmount();
  });

  it("draws every conversation flat, with no heading for where it lives", () => {
    const view = mount(
      <ActivityRail {...props([
        thread("in-workspace", { mode: "workspace", title: "In a workspace" }),
        thread("a-chat", { mode: "chat", title: "A chat" }),
      ])}
      />,
    );
    showRecent(view.container);
    // The tree's headings are gone: the list is the flat rows and their two
    // captions, with no workspace heading and no chats heading above them.
    const list = view.container.querySelector("[data-testid=\"recent-rows\"]")!;
    expect(list).not.toBeNull();
    expect(captions(view.container)).toEqual(["earlier:Earlier:2"]);
    expect(drawnTitles(view.container)).toEqual(["A chat", "In a workspace"]);
    // The only "Workspaces" left on screen is the switch's own label.
    expect(list.textContent).not.toContain("Workspaces");
    view.unmount();
  });
});

describe("the recent view's order", () => {
  it("puts the runs in flight above the rest, ranked by when they started", () => {
    const view = mount(
      <ActivityRail {...props(
        [
        // Started first, spoke most recently: by activity it would lead, by
        // creation it is the last of the running rows. This is the jump the
        // ordering exists to remove.
          thread("busy-old", { title: "Busy old", createdAt: now - 3 * hour, lastMessageAt: now - 1_000 }),
          thread("busy-new", { title: "Busy new", createdAt: now - minute, lastMessageAt: now - minute }),
          // A run waiting on the reader: not streaming, and the first thing the
          // view should offer once it is the newest conversation of the bunch.
          thread("approval", { title: "Approval", createdAt: now - minute / 2, lastMessageAt: now - 30 * minute }),
          thread("idle-new", { title: "Idle new", createdAt: now - 10 * minute, lastMessageAt: now - 2 * minute }),
          thread("idle-old", { title: "Idle old", createdAt: now - 5 * hour, lastMessageAt: now - 90 * minute }),
        ],
        {
          threadRunStatuses: {
            "busy-old": { status: "running", endedAt: null },
            "busy-new": { status: "running", endedAt: null },
            "approval": { status: "waiting_approval", endedAt: null },
          },
        },
      )}
      />,
    );
    showRecent(view.container);

    expect(captions(view.container)).toEqual(["active:Active:3", "earlier:Earlier:2"]);
    expect(drawnTitles(view.container)).toEqual([
      // Active, by start time: the approval row started after both runs.
      "Approval",
      "Busy new",
      "Busy old",
      // Earlier, by last reply.
      "Idle new",
      "Idle old",
    ]);
    view.unmount();
  });

  it("follows a run the agent reports as streaming without a local run row", () => {
    const view = mount(
      <ActivityRail {...props(
        [thread("tui-run", { title: "Started elsewhere", lastMessageAt: now - 5 * minute })],
        { threadStreamingStatuses: { "tui-run": true } },
      )}
      />,
    );
    showRecent(view.container);
    expect(captions(view.container)).toEqual(["active:Active:1"]);
    view.unmount();
  });

  it("keeps a pinned conversation in its place, with its marker still drawn", () => {
    const view = mount(
      <ActivityRail {...props([
        thread("pinned-stale", { title: "Pinned stale", pinned: true, lastMessageAt: now - 90 * minute }),
        thread("fresh", { title: "Fresh", lastMessageAt: now - minute }),
      ])}
      />,
    );
    showRecent(view.container);
    // No pinned section: this view ranks by one clock and draws the pin as a
    // marker, so the row is not lifted to the top.
    expect(captions(view.container)).toEqual(["earlier:Earlier:2"]);
    expect(drawnTitles(view.container)).toEqual(["Fresh", "Pinned stale"]);
    view.unmount();
  });

  it("keeps this machine's order when a paired machine's rows are in the list", () => {
    const view = mount(
      <ActivityRail {...props(
        [thread("local", { title: "Local", lastMessageAt: now - 10 * minute })],
        {
          remoteCatalogs: [{
            desktopId: "desk-b",
            sessions: [{
              sessionId: "remote",
              title: "Remote",
              mode: "chat",
              streaming: true,
              // Started later than the local row: it leads the active stretch.
              createdAt: now - minute,
              lastMessageAt: now - 20_000,
            }],
          }],
        },
      )}
      />,
    );
    showRecent(view.container);
    expect(captions(view.container)).toEqual(["active:Active:1", "earlier:Earlier:1"]);
    // The remote row names its machine in its own label: in a flat list of
    // several machines' conversations, which pane a row came from is not
    // something a heading above it can say.
    expect(drawnTitles(view.container)).toEqual(["Remote · desk-b", "Local"]);
    view.unmount();
  });
});

describe("the recent view's window", () => {
  it("draws one window of the finished rows and opens the rest on request", () => {
    const many = [
      thread("busy", { title: "Busy", lastMessageAt: now }),
      ...Array.from({ length: RECENT_PAGE_SIZE + 12 }, (_, index) =>
        thread(`idle-${index}`, {
          title: `Idle ${index}`,
          createdAt: now - (index + 2) * minute,
          lastMessageAt: now - (index + 1) * minute,
        })),
    ];
    const view = mount(
      <ActivityRail {...props(many, {
        threadRunStatuses: { busy: { status: "running", endedAt: null } },
      })}
      />,
    );
    showRecent(view.container);

    expect(drawnTitles(view.container)).toHaveLength(RECENT_PAGE_SIZE + 1);
    const more = view.container.querySelector<HTMLButtonElement>("[data-testid=\"recent-show-more\"]")!;
    expect(more.textContent).toContain("12");
    expect(more.getAttribute("aria-label")).toContain("12");

    act(() => more.click());
    expect(drawnTitles(view.container)).toHaveLength(RECENT_PAGE_SIZE + 13);
    // Nothing left to offer, so the control goes with it.
    expect(view.container.querySelector("[data-testid=\"recent-show-more\"]")).toBeNull();
    view.unmount();
  });
});

describe("a batch in the recent view", () => {
  it("reaches a pinned conversation, which the tree leaves out", () => {
    // The tree lifts a pin above its groups, so a group's "select all" must not
    // sweep it up and it carries no checkbox. This view opens no such section —
    // the pin is a marker — so the row is one of the list's own rows, and leaving
    // it unselectable would put one row of this very list beyond the batch.
    const threads = [
      thread("pinned", { title: "Pinned one", pinned: true, lastMessageAt: now - minute }),
      thread("plain", { title: "Plain one", lastMessageAt: now - hour }),
    ];
    const view = mount(<ActivityRail {...props(threads)} />);

    // The tree: the pinned row is its own section, outside every group, so the
    // chat section's batch leaves it out.
    act(() => view.container.querySelector<HTMLButtonElement>("[aria-label=\"Chat section actions\"]")!.click());
    act(() => [...view.container.querySelectorAll<HTMLElement>("button")]
      .find(element => element.textContent === "Select chats")!
      .click());
    expect(rowCheckboxes(view.container)).toEqual(["Select Plain one"]);
    view.unmount();
  });

  /**
   * The recent list has no batch of its own.
   *
   * A batch is "the rows under this heading" — a workspace, the chats section —
   * and this list's only boundary is time, so a selection here would mean every
   * conversation on the machine. Nothing above the list offers one either, so a
   * conversation is removed from this view by its own row's menu.
   */
  it("offers no batch entry anywhere while the recent view is the list", () => {
    const threads = [
      thread("pinned", { title: "Pinned one", pinned: true, lastMessageAt: now - minute }),
      thread("plain", { title: "Plain one", lastMessageAt: now - hour }),
    ];
    const view = mount(<ActivityRail {...props(threads)} />);
    showRecent(view.container);

    expect(view.container.querySelector("button[aria-label='Recent list actions']")).toBeNull();
    // Both rows are drawn, and each keeps its own menu — that is how something
    // leaves this list.
    expect(rowCheckboxes(view.container)).toEqual([]);
    expect(view.container.querySelector("button[aria-label='Thread actions for Plain one']")).not.toBeNull();
    view.unmount();
  });
});

/**
 * Opening a conversation from this list keeps the reader in this list.
 *
 * Reported as "in the recent list, clicking a conversation jumps the sidebar to
 * the workspaces". The cause was that opening a conversation moved the list
 * selection along with it: the shell sets its section from the conversation's own
 * mode, and the rail followed that into the workspace list — so the row the
 * reader had just clicked disappeared from under the pointer. The list they
 * chose is theirs to change.
 *
 * The shell's half of that is asserted here too: `active` goes to "workspace"
 * (what the shell does for a workspace conversation) and the drawn list must not
 * move with it.
 */
describe("opening a conversation from the recent list", () => {
  it("keeps the recent list, whatever list the conversation belongs to", () => {
    const threads = [
      thread("in-workspace", { mode: "workspace", title: "In a workspace", workspaceId: "ws" }),
      thread("a-chat", { mode: "chat", title: "A chat" }),
    ];
    // A real workspace, so the tree half has something to draw: without one, a
    // regression that rendered both halves at once would look like a clean recent
    // list.
    const workspaces = [workspace("ws", "Charts")];
    const view = mount(<ActivityRail {...props(threads, { active: "chat", workspaces })} />);
    showRecent(view.container);
    expect(drawnTitles(view.container)).toEqual(["A chat", "In a workspace"]);

    // The shell selects the workspace conversation, and its section follows the
    // conversation's mode — the exact condition that used to move the list.
    view.rerender(
      <ActivityRail {...props(threads, { active: "workspace", activeThreadId: "in-workspace", workspaces })} />,
    );

    // Still the recent list, and *only* it: same rows, same caption, and none of
    // the tree's own furniture (a workspace group heading above the rows).
    expect(drawnTitles(view.container)).toEqual(["A chat", "In a workspace"]);
    expect(captions(view.container)).toEqual(["earlier:Earlier:2"]);
    expect(view.container.querySelector("[data-testid='local-group-heading']")).toBeNull();
    expect(view.container.querySelector("button[aria-label='Collapse workspace']")).toBeNull();
    view.unmount();
  });
});

describe("the recent view from the keyboard", () => {
  it("walks the drawn order, not the tree's", async () => {
    // The rail's walk and the list it walks are one order: a view that drew one
    // list and stepped another would move the selection somewhere the reader
    // cannot see. The two orders must *differ* in this fixture or a walk that
    // still used the tree's would pass: `busy` started last and spoke longest
    // ago, so it is last in the tree and first here.
    resetAllCommandBindings();
    const uninstallShortcuts = installShortcuts();
    const onSelectThread = vi.fn();
    const view = mount(
      <ActivityRail {...props(
        [
          thread("busy", { title: "Busy", createdAt: now - minute, lastMessageAt: now - 3 * hour }),
          thread("fresh", { title: "Fresh", createdAt: now - 5 * hour, lastMessageAt: now - minute }),
          thread("idle-old", { title: "Idle old", createdAt: now - 6 * hour, lastMessageAt: now - 5 * hour }),
        ],
        { onSelectThread, threadRunStatuses: { busy: { status: "running", endedAt: null } } },
      )}
      />,
    );
    showRecent(view.container);

    const press = (init: KeyboardEventInit) => act(() => {
      window.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, cancelable: true, ...init }));
    });
    // Nothing selected: the fine move starts at the first drawn row, which is the
    // active one — the same order the list draws.
    press({ code: "BracketRight", ctrlKey: true, key: "]", shiftKey: true });
    expect(onSelectThread).toHaveBeenLastCalledWith(expect.objectContaining({ id: "busy" }));

    // From the active row, the next stop is the finished one under it.
    view.rerender(
      <ActivityRail {...props(
        [
          thread("busy", { title: "Busy", createdAt: now - minute, lastMessageAt: now - 3 * hour }),
          thread("fresh", { title: "Fresh", createdAt: now - 5 * hour, lastMessageAt: now - minute }),
          thread("idle-old", { title: "Idle old", createdAt: now - 6 * hour, lastMessageAt: now - 5 * hour }),
        ],
        {
          activeThreadId: "busy",
          onSelectThread,
          threadRunStatuses: { busy: { status: "running", endedAt: null } },
        },
      )}
      />,
    );
    press({ code: "BracketRight", ctrlKey: true, key: "]", shiftKey: true });
    // The next stop is the row drawn under it — the newest *finished* conversation,
    // which is the tree's first row.
    expect(onSelectThread).toHaveBeenLastCalledWith(expect.objectContaining({ id: "fresh" }));

    view.unmount();
    uninstallShortcuts();
    resetAllCommandBindings();
  });
});

describe("the recent view and the tree do not leak into each other", () => {
  it("leaves the tree's order and grouping exactly as they were", () => {
    const threads = [
      thread("pinned", { title: "Pinned one", pinned: true, lastMessageAt: now - hour }),
      thread("ws-new", {
        mode: "workspace",
        workspaceId: "ws-alpha",
        title: "Workspace newest",
        lastMessageAt: now - minute,
      }),
      thread("ws-old", {
        mode: "workspace",
        workspaceId: "ws-alpha",
        title: "Workspace older",
        lastMessageAt: now - 2 * hour,
      }),
    ];
    const workspaces = [{
      cleanupStatus: "active" as const,
      createdAt: 0,
      id: "ws-alpha",
      kind: "user" as const,
      name: "Alpha",
      path: "/tmp/alpha",
      updatedAt: now,
    }];
    const view = mount(
      <ActivityRail {...props(threads, {
      // A run in flight, which the recent view would lift to the top. It must not
      // move inside the tree.
        threadRunStatuses: { "ws-old": { status: "running", endedAt: null } },
        workspaces,
      })}
      />,
    );

    // The tree: the pinned row above the workspace group, and the group's rows in
    // their own recency order — the running one stays second.
    const treeText = view.container.textContent ?? "";
    expect(treeText.indexOf("Pinned one")).toBeLessThan(treeText.indexOf("Workspace newest"));
    expect(treeText.indexOf("Workspace newest")).toBeLessThan(treeText.indexOf("Workspace older"));

    // And switching away and back changes nothing about it.
    showRecent(view.container);
    act(() => view.container.querySelector<HTMLButtonElement>("[data-testid=\"rail-view-workspace\"]")!.click());
    const again = view.container.textContent ?? "";
    expect(again.indexOf("Pinned one")).toBeLessThan(again.indexOf("Workspace newest"));
    expect(again.indexOf("Workspace newest")).toBeLessThan(again.indexOf("Workspace older"));
    view.unmount();
  });

  it("remembers the view across mounts", () => {
    const first = mount(<ActivityRail {...props([thread("a")])} />);
    showRecent(first.container);
    first.unmount();

    const second = mount(<ActivityRail {...props([thread("a")])} />);
    expect(second.container.querySelector("[data-testid=\"recent-rows\"]")).not.toBeNull();
    expect(second.container.querySelector("[data-testid=\"rail-view-recent\"]")!.getAttribute("aria-pressed"))
      .toBe("true");
    second.unmount();
  });
});
