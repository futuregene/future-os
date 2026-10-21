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

function band(container: HTMLElement, id: "rail-view-band" | "rail-list-tools") {
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

describe("the rail's bands", () => {
  beforeEach(() => {
    localStorage.clear();
    document.body.innerHTML = "";
  });

  /**
   * Band 1 answers one question and holds nothing else — in **every** tab.
   *
   * The menu used to sit here, and only in one tab; reading the band in one tab
   * is what let that pass unnoticed, so this walks all three.
   */
  it.each(TABS)("keeps the switch's band to the switch alone (%s)", async (tab) => {
    const { container } = await render(<Rail {...props()} />);
    act(() => [...container.querySelectorAll<HTMLButtonElement>("[data-testid^='rail-view-']")]
      .find(button => button.title === tab)!
      .click());
    await flushAsync();
    expect(container.querySelector<HTMLButtonElement>("[data-testid='rail-view-switch'] button[aria-pressed='true']")!.title).toBe(tab);

    const viewBand = band(container, "rail-view-band")!;
    expect(viewBand.querySelector("[data-testid='rail-view-switch']")).not.toBeNull();
    // Nothing that belongs to the list: no search field, no list menu, no menu
    // trigger of any kind.
    expect(viewBand.querySelector("input")).toBeNull();
    expect(viewBand.querySelector("button[aria-haspopup='menu']")).toBeNull();
    // The two lists that have a boundary keep their own menu — otherwise those
    // assertions would be about a control that is not on screen at all. The
    // recent list has none (see the "boundary" test below).
    expect(listMenu(container, tab) === null).toBe(tab === "Recent");
  });

  it("keeps the search field on screen, in the list's own band, with no interaction", async () => {
    const { container } = await render(<Rail {...props()} />);
    const tools = band(container, "rail-list-tools")!;
    const field = searchField(tools);
    expect(field).not.toBeNull();
    // Drain the click-into-a-mode path: the field used to appear only after a
    // glyph was clicked. If it is here with nothing done, that path is gone.
    expect(field!.value).toBe("");
    expect(band(container, "rail-view-band")!.contains(field)).toBe(false);
  });

  /**
   * One row holds the search field and the list's own controls.
   *
   * The row is where the field lives (it filters the list drawn under it), and
   * the list's menu — where the list has one — sits at that row's right end.
   */
  it.each(TABS)("keeps the search field in the list's row (%s)", async (tab) => {
    const { container } = await render(<Rail {...props()} />);
    act(() => [...container.querySelectorAll<HTMLButtonElement>("[data-testid^='rail-view-']")]
      .find(button => button.title === tab)!
      .click());
    await flushAsync();

    const tools = band(container, "rail-list-tools")!;
    expect(tools.contains(searchField(tools))).toBe(true);

    const trigger = listMenu(tools, tab);
    if (tab === "Recent") {
      expect(trigger).toBeNull();
    }
    else {
      expect(trigger).not.toBeNull();
      // At the row's right end, after the field.
      expect(tools.lastElementChild!.contains(trigger)).toBe(true);
    }
  });

  it("orders the band: machine picker, search, the tab's create control, then the menu", async () => {
    const p = props();
    p.remotePeers = [peer];
    p.onChangeDeviceFilter = vi.fn();
    p.onManageDesktops = vi.fn();
    const { container } = await render(<Rail {...p} />);

    const tools = band(container, "rail-list-tools")!;
    const children = Array.from(tools.children);
    expect(children[0]!.textContent).toContain("All devices");
    expect(children[1]!.contains(searchField(tools))).toBe(true);
    // The workspace tab's "+" (new workspace) sits between them and the menu.
    // The "+" is the band's own item, not something wrapped around one.
    expect(children[2]!.getAttribute("aria-label")).toBe("New workspace");
    expect(children[3]!.contains(listMenu(tools, "Workspaces"))).toBe(true);
  });

  it("drops the list's band while a batch is being selected", async () => {
    const { container } = await render(<Rail {...props()} />);
    act(() => container.querySelector<HTMLButtonElement>("button[aria-label='Workspaces list actions']")!.click());
    await flushAsync();
    act(() => [...container.querySelectorAll<HTMLButtonElement>("button")]
      .find(button => button.textContent?.includes("Select conversations in “Workspaces”"))!
      .click());
    await flushAsync();

    // The selection toolbar replaces the list's controls; the switch stays, since
    // which list the batch covers is still the question it answers.
    expect(band(container, "rail-list-tools")).toBeNull();
    expect(band(container, "rail-view-band")).not.toBeNull();
  });

  /**
   * The recent list draws no list-wide menu, and keeps the rest of its row.
   *
   * A batch is "the rows under this heading" — a workspace, the chats list. This
   * list's only boundary is time, so a selection here would mean every
   * conversation on the machine, which is not a batch a reader can picture. The
   * row itself stays, because the search field lives in it: narrowing the recent
   * list is exactly what a reader does with it.
   */
  it("draws no menu on the recent list, and keeps its search field", async () => {
    const { container } = await render(<Rail {...props()} />);
    act(() => container.querySelector<HTMLButtonElement>("[data-testid='rail-view-recent']")!.click());
    await flushAsync();

    expect(listMenu(container, "Recent")).toBeNull();
    expect(searchField(container)).not.toBeNull();
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
