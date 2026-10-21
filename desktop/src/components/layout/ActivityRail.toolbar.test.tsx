// @vitest-environment jsdom
import type { ComponentProps, ReactElement } from "react";
import type { StoredWorkspace } from "../../integrations/storage/threadStore";
import { act, useState } from "react";
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
    active: "conversations",
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
    view: "workspace",
    onSelectView: vi.fn(),
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

async function render(node: ReactElement) {
  const container = document.createElement("div");
  document.body.appendChild(container);
  const root = createRoot(container);
  act(() => root.render(node));
  await flushAsync();
  return { container, root };
}

function band(container: HTMLElement, id: "rail-search-band" | "rail-view-band" | "rail-list-actions") {
  return container.querySelector<HTMLElement>(`[data-testid="${id}"]`);
}

const searchField = (root: HTMLElement) => root.querySelector<HTMLInputElement>("input[aria-label='Search conversations']");
/** Each tab's list menu, by the name the tab gives it. */
function listMenu(root: HTMLElement, view: "Workspaces" | "Chats" | "Recent") {
  return root.querySelector<HTMLButtonElement>(`button[aria-label='${view} list actions']`);
}

/**
 * The rail, with the tab held the way the shell holds it.
 *
 * `view` is the shell's state now (so opening a conversation can follow it), which
 * means a click on a tab needs something to answer. This is that something, and
 * nothing more: it is the smallest shell that can hold a tab.
 */
function Rail(base: ComponentProps<typeof ActivityRail>) {
  const [view, setView] = useState(base.view);
  return (
    <ActivityRail
      {...base}
      view={view}
      onSelectView={(next) => {
        base.onSelectView?.(next);
        setView(next);
      }}
    />
  );
}

const TABS = ["Workspaces", "Chats", "Recent"] as const;

/** Switch to a tab the way a reader does. */
async function showTab(container: HTMLElement, tab: typeof TABS[number]) {
  act(() => [...container.querySelectorAll<HTMLButtonElement>("[data-testid^='rail-view-']")]
    .find(button => button.title === tab)!
    .click());
  await flushAsync();
}

describe("the rail's bands", () => {
  beforeEach(() => {
    localStorage.clear();
    document.body.innerHTML = "";
  });

  /**
   * The row order, top to bottom: the filter, then which list, then the list's
   * own actions.
   *
   * The search and the machine picker sit *above* the list switch — above the
   * workspace row — directly over the list they narrow, and that is where they
   * have always been. A regression here is a control that filters a list from
   * underneath the list's own switch, or a switch that no longer sits between the
   * filter and the list.
   */
  it("stacks the filter above the list switch, and the list's actions below it", async () => {
    const p = props();
    p.remotePeers = [peer];
    p.onChangeDeviceFilter = vi.fn();
    p.onManageDesktops = vi.fn();
    const { container } = await render(<Rail {...p} />);

    const search = band(container, "rail-search-band")!;
    const view = band(container, "rail-view-band")!;
    const actions = band(container, "rail-list-actions")!;
    expect(search).not.toBeNull();
    expect(view).not.toBeNull();
    expect(actions).not.toBeNull();

    // In that order in the document: the filter, the switch, the actions.
    expect(search.compareDocumentPosition(view) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(view.compareDocumentPosition(actions) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    // And all three above the list they describe.
    const list = container.querySelector("[data-activity-rail-scroll]")!;
    expect(actions.compareDocumentPosition(list) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();

    // The machine picker and the search share the top row.
    expect(search.contains(container.querySelector("button[aria-label='All devices']"))).toBe(true);
    expect(search.contains(searchField(container))).toBe(true);
  });

  /**
   * Band 1 answers one question and holds nothing else — in **every** tab.
   *
   * The menu used to sit here, and only in one tab; reading the band in one tab
   * is what let that pass unnoticed, so this walks all three.
   */
  it.each(TABS)("keeps the switch's band to the switch alone (%s)", async (tab) => {
    const { container } = await render(<Rail {...props()} />);
    await showTab(container, tab);
    expect(container.querySelector<HTMLButtonElement>("[data-testid='rail-view-switch'] button[aria-pressed='true']")!.title).toBe(tab);

    const viewBand = band(container, "rail-view-band")!;
    expect(viewBand.querySelector("[data-testid='rail-view-switch']")).not.toBeNull();
    // Nothing that belongs to the list: no search field, no list menu, no menu
    // trigger of any kind. The filter has its own row above.
    expect(viewBand.querySelector("input")).toBeNull();
    expect(viewBand.querySelector("button[aria-haspopup='menu']")).toBeNull();
  });

  it("keeps the search field on screen, above the switch, with no interaction", async () => {
    const { container } = await render(<Rail {...props()} />);
    const field = searchField(band(container, "rail-search-band")!);
    expect(field).not.toBeNull();
    // Drain the click-into-a-mode path: the field used to appear only after a
    // glyph was clicked. If it is here with nothing done, that path is gone.
    expect(field!.value).toBe("");
    expect(band(container, "rail-view-band")!.contains(field)).toBe(false);
  });

  it.each(TABS)("filters the list whose switch sits under it (%s)", async (tab) => {
    const { container } = await render(<Rail {...props()} />);
    await showTab(container, tab);
    // The filter is above the switch in every tab — it narrows whichever list is
    // drawn, so it cannot be a control of one of them.
    expect(searchField(container)).not.toBeNull();
  });

  /**
   * The list's own actions: on the two lists that have a boundary, and not on the
   * recent one.
   *
   * "Everything, by time" is not a batch a reader can picture — the workspace and
   * the chats list are where a reader goes to delete something — so the recent tab
   * draws no trigger rather than one that promises a selection it cannot
   * describe. With no batch and nothing to add to that list, its action row is not
   * drawn at all.
   */
  it.each(TABS)("offers the list's own menu only where the list has a boundary (%s)", async (tab) => {
    const { container } = await render(<Rail {...props()} />);
    await showTab(container, tab);

    const trigger = listMenu(container, tab);
    if (tab === "Recent") {
      expect(trigger).toBeNull();
      expect(band(container, "rail-list-actions")).toBeNull();
    }
    else {
      expect(trigger).not.toBeNull();
      const actions = band(container, "rail-list-actions")!;
      expect(actions.contains(trigger)).toBe(true);
      // At the row's right end.
      expect(actions.lastElementChild!.contains(trigger)).toBe(true);
    }
  });

  it("orders the action row: the tab's create control, then the list's menu", async () => {
    const p = props();
    p.remotePeers = [peer];
    p.onChangeDeviceFilter = vi.fn();
    p.onManageDesktops = vi.fn();
    const { container } = await render(<Rail {...p} />);

    const actions = band(container, "rail-list-actions")!;
    const children = Array.from(actions.children);
    // The workspace tab's "+" (new workspace) first, then the menu.
    expect(children[0]!.getAttribute("aria-label")).toBe("New workspace");
    expect(children[1]!.contains(listMenu(actions, "Workspaces"))).toBe(true);
  });

  it("drops the list's action row while a batch is being selected", async () => {
    const { container } = await render(<Rail {...props()} />);
    act(() => listMenu(container, "Workspaces")!.click());
    await flushAsync();
    act(() => [...container.querySelectorAll<HTMLButtonElement>("button")]
      .find(button => button.textContent?.includes("Select conversations in “Workspaces”"))!
      .click());
    await flushAsync();

    // The selection toolbar replaces them; the switch and the filter row stay —
    // which list the batch covers, and which machine's rows, are still asked.
    expect(band(container, "rail-list-actions")).toBeNull();
    expect(band(container, "rail-view-band")).not.toBeNull();
  });

  /**
   * The recent list draws no list-wide menu, and keeps the rows above it.
   *
   * A batch is "the rows under this heading" — a workspace, the chats list. This
   * list's only boundary is time, so a selection here would mean every
   * conversation on the machine, which is not a batch a reader can picture. The
   * search row stays where it is: narrowing the recent list is exactly what a
   * reader does with it, and its position is unchanged by this.
   */
  it("draws no menu on the recent list, and keeps the search where it is", async () => {
    const { container } = await render(<Rail {...props()} />);
    await showTab(container, "Recent");

    expect(listMenu(container, "Recent")).toBeNull();
    const search = band(container, "rail-search-band")!;
    expect(searchField(search)).not.toBeNull();
    // Still above the switch, not moved down into the gap the menu left.
    expect(search.compareDocumentPosition(band(container, "rail-view-band")!) & Node.DOCUMENT_POSITION_FOLLOWING)
      .toBeTruthy();
  });

  /**
   * The chat tab keeps the machine-aware "new conversation".
   *
   * It lived on the chats section heading, which the tabs replaced; a
   * conversation with no workspace can only be started on a chosen machine, and
   * dropping the control with its heading would have made that impossible.
   */
  it("keeps the machine-aware new conversation on the chats tab, and only there", async () => {
    const p = props();
    p.remotePeers = [peer];
    p.onChangeDeviceFilter = vi.fn();
    p.onManageDesktops = vi.fn();
    const { container } = await render(<Rail {...p} />);
    const newChatOn = () => container.querySelector("button[aria-label='New chat']");

    // Workspaces tab: creating a *workspace* is the action; a machine-less chat
    // is the top nav's, which is outside this band.
    expect(newChatOn()).toBeNull();
    act(() => container.querySelector<HTMLButtonElement>("[data-testid='rail-view-chat']")!.click());
    await flushAsync();
    expect(newChatOn()).not.toBeNull();
    act(() => container.querySelector<HTMLButtonElement>("[data-testid='rail-view-recent']")!.click());
    await flushAsync();
    expect(newChatOn()).toBeNull();
  });
});

/**
 * Three lists, and the tab is what decides which one is drawn.
 *
 * The structural claim under test: a workspace conversation and a chat are
 * siblings, so *neither* list may draw the other's rows. Before the tabs they
 * were two sections of one list drawn from the shell's page (`ActivitySection`),
 * which is why the chats read as living inside the workspaces — an arrangement
 * that satisfies "both kinds are on screen" and not "this is the list I chose".
 */
describe("the three lists", () => {
  beforeEach(() => {
    localStorage.clear();
    document.body.innerHTML = "";
  });

  const fixture = () => {
    const p = props();
    p.threads = [
      { ...thread("chat-one", "A chat"), mode: "chat" },
      { ...thread("ws-one", "A workspace conversation"), mode: "workspace", workspaceId: "ws-1" },
    ];
    p.workspaces = [{ id: "ws-1", name: "Charts", kind: "user", path: "/tmp/ws-1", cleanupStatus: "active", createdAt: 0, updatedAt: 0 }];
    return p;
  };

  it("draws the three tabs in the order the design names them", async () => {
    const { container } = await render(<Rail {...props()} />);
    const tabs = [...container.querySelectorAll<HTMLButtonElement>("[data-testid='rail-view-switch'] button")];
    expect(tabs.map(tab => tab.getAttribute("data-testid"))).toEqual([
      "rail-view-workspace",
      "rail-view-chat",
      "rail-view-recent",
    ]);
    // The names are on the button, not only in its tooltip: three glyphs with no
    // words would have made the reader hover to learn what each one is.
    expect(tabs.map(tab => tab.textContent?.trim())).toEqual(["Workspaces", "Chats", "Recent"]);
  });

  it("draws the workspace list, and nothing from the chats list", async () => {
    const { container } = await render(<Rail {...fixture()} />);

    expect(container.querySelector("[data-testid='workspace-tab']")).not.toBeNull();
    expect(container.textContent).toContain("A workspace conversation");
    // The regression this exists for: a workspace conversation and a chat in one
    // list is what made "in the workspaces" ambiguous — and it is what a revert
    // to drawing the shell's two sections produces.
    expect(container.textContent).not.toContain("A chat");
    expect(container.querySelector("[data-testid='chat-tab']")).toBeNull();
  });

  it("draws the chats list, and nothing from the workspace list", async () => {
    const p = { ...fixture(), view: "chat" as const };
    const { container } = await render(<Rail {...p} />);

    expect(container.querySelector("[data-testid='chat-tab']")).not.toBeNull();
    expect(container.textContent).toContain("A chat");
    expect(container.textContent).not.toContain("A workspace conversation");
    expect(container.querySelector("[data-testid='workspace-tab']")).toBeNull();
  });

  /**
   * The page and the list are two questions.
   *
   * The list is drawn whichever page the window is on — the rail is always the
   * rail — so its contents must come from the tab alone. A shell's page value
   * leaning on the list's choice is the old coupling, and it is what this test
   * would catch coming back.
   */
  it("draws the chosen list whatever page the window is on", async () => {
    const p = { ...fixture(), active: "skill" as const, view: "chat" as const };
    const { container } = await render(<Rail {...p} />);
    expect(container.textContent).toContain("A chat");
    expect(container.textContent).not.toContain("A workspace conversation");
  });

  /** A pin is a shortcut *within* its list, so it lands in its own tab's pinned section. */
  it("puts each pinned conversation in its own list's pinned section", async () => {
    const p = fixture();
    p.view = "chat";
    p.threads = [
      { ...p.threads![0]!, pinned: true },
      { ...p.threads![1]!, pinned: true },
    ];
    const { container } = await render(<Rail {...p} />);
    // The chat tab's pinned section holds the pinned chat…
    expect(container.textContent).toContain("Pinned");
    expect(container.textContent).toContain("A chat");
    // …and not the pinned workspace conversation, which belongs to the other list.
    expect(container.textContent).not.toContain("A workspace conversation");
  });
});
