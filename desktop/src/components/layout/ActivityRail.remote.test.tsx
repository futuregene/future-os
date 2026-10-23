// @vitest-environment jsdom
import type { ComponentProps } from "react";
import type { StoredThread, StoredWorkspace } from "../../integrations/storage/threadStore";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { resetAllCommandBindings } from "../../features/keyboard/shortcuts";
import { installShortcuts } from "../../test/installShortcuts";
import { flushAsync } from "../../test/renderHook";
import { ActivityRail } from "./ActivityRail";

/**
 * The rail's client-role behaviour: which machine's conversations it lists, and
 * what it does when the user acts on one.
 *
 * Asserted through the rail rather than the row components alone because the
 * wiring between them is where a remote action could be handed to the wrong
 * handler — a rename that opens the *local* dialog, or an action that never
 * triggers a re-read of the host.
 */

const pin = vi.fn<(desktopId: string, address: unknown, pinned: boolean) => Promise<unknown>>();
const remove = vi.fn<(desktopId: string, address: unknown) => Promise<unknown>>();
vi.mock("../../features/remote-peer/remotePeerClient", () => ({
  deleteRemoteConversation: (...args: Parameters<typeof remove>) => remove(...args),
  pinRemoteConversation: (...args: Parameters<typeof pin>) => pin(...args),
  renameRemoteConversation: vi.fn(async () => {}),
}));

vi.mock("../../integrations/skills/skillsClient", () => ({ listInstalledSkills: async () => [] }));
vi.mock("../../integrations/agent/agentStateCache", () => ({ useCachedAgentState: () => undefined }));
vi.mock("../../lib/useIsFullscreen", () => ({ useIsFullscreen: () => false }));
vi.mock("./hooks/usePendingApprovalCounts", () => ({ usePendingApprovalCounts: () => new Map() }));
vi.mock("../../lib/useFloatingScrollbar", () => ({
  useFloatingScrollbar: () => ({
    handleScroll: () => {},
    handleThumbPointerDown: () => {},
    scrollRef: { current: null },
    scrollbar: { height: 0, top: 0, visible: false },
  }),
}));

function thread(id: string, overrides: Partial<StoredThread> = {}): StoredThread {
  return {
    id,
    agentSessionId: id,
    createdAt: 1,
    mode: "chat",
    pinned: false,
    readonly: false,
    status: "active",
    title: id,
    updatedAt: 1,
    workspaceId: id,
    ...overrides,
  };
}

const peer = {
  agentAvailable: true,
  bridgeInstanceId: "bridge_1",
  connected: true,
  stopped: false,
  desktopId: "desktop_a",
  error: null,
  features: [],
  icon: "laptop",
  name: "Studio iMac",
  pairId: "pair_1",
};

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

function railProps(overrides: Partial<ComponentProps<typeof ActivityRail>> = {}) {
  return {
    active: "chat" as const,
    activeThreadId: null,
    expanded: true,
    onBatchDeleteThreads: vi.fn(),
    onDeleteRemoteConversations: vi.fn(),
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
    threads: [] as StoredThread[],
    unreadThreadIds: new Set<string>(),
    workspaces: [],
    ...overrides,
  } as ComponentProps<typeof ActivityRail>;
}

/** A host snapshot with one conversation, so the list enters merged mode. */
function catalogRow(sessionId: string, overrides: Record<string, unknown> = {}) {
  return {
    desktopId: "desktop_a",
    sessions: [{
      lastMessageAt: 5_000,
      sessionId,
      threadId: `${sessionId}_thread`,
      title: "Remote conversation",
      ...overrides,
    }],
  } as never;
}

async function mountRail(overrides: Partial<ComponentProps<typeof ActivityRail>> = {}) {
  const container = document.createElement("div");
  document.body.appendChild(container);
  const root = createRoot(container);
  act(() => root.render(<ActivityRail {...railProps(overrides)} />));
  await flushAsync();
  return {
    container,
    root,
    /** The overflow control of the remote row, revealed on hover in the product. */
    actionsButton: () => container.querySelector<HTMLButtonElement>("button[aria-label='More actions']"),
    item: (label: string) => [...container.querySelectorAll("button")]
      .find(node => node.textContent === label),
    /** The remote nav dot's own state, or null when it is not drawn. */
    dotState: () => container.querySelector("[data-testid='remote-status-dot']")?.getAttribute("data-state") ?? null,
    text: () => container.textContent ?? "",
    unmount: () => {
      act(() => root.unmount());
      container.remove();
    },
  };
}

describe("the remote nav dot", () => {
  /**
   * With machines paired, the dot answers for the *set*.
   *
   * Nothing about one machine can express "the merged list is short of another":
   * the list interleaves every host's conversations, so the only honest summary
   * is how many of them are up.
   */
  it("is green only when every paired machine is up", async () => {
    const view = await mountRail({
      remotePeers: [peer, { ...peer, desktopId: "desktop_b" }],
    });
    expect(view.dotState()).toBe("up");
    view.unmount();
  });

  it("is amber when some of them are", async () => {
    const view = await mountRail({
      remotePeers: [peer, { ...peer, connected: false, desktopId: "desktop_b", error: "gone" }],
    });
    expect(view.dotState()).toBe("partial");
    view.unmount();
  });

  it("is red when none of them are", async () => {
    const view = await mountRail({
      remotePeers: [{ ...peer, connected: false, error: "gone" }],
    });
    expect(view.dotState()).toBe("down");
    view.unmount();
  });

  /**
   * With nothing paired it keeps its other meaning — this machine's own host
   * bridge, which is how devices connect *in* — so an unpaired machine still
   * reports whether remote access itself is up.
   */
  it("falls back to the host bridge's own state when nothing is paired", async () => {
    const view = await mountRail({ remoteIndicator: "connected", remotePeers: [] });
    expect(view.dotState()).toBe("up");
    view.unmount();
  });

  it("has no dot before anything is paired", async () => {
    const view = await mountRail({ remotePeers: [] });
    expect(view.dotState()).toBeNull();
    view.unmount();
  });

  /** A stopped machine is still a machine that is not answering. */
  it("counts a machine the user stopped as not up", async () => {
    const view = await mountRail({
      remotePeers: [peer, { ...peer, connected: false, desktopId: "desktop_b", stopped: true }],
    });
    expect(view.dotState()).toBe("partial");
    view.unmount();
  });
});

async function settle(): Promise<void> {
  await act(async () => {
    for (let i = 0; i < 4; i += 1) await Promise.resolve();
  });
}

beforeEach(() => {
  document.body.innerHTML = "";
  localStorage.clear();
  pin.mockReset().mockResolvedValue(undefined);
  remove.mockReset().mockResolvedValue(undefined);
});

describe("the rail's remote conversations", () => {
  it("lists a host's conversation and opens it on that host", async () => {
    const onOpenRemoteConversation = vi.fn();
    const view = await mountRail({
      deviceFilter: { kind: "all" },
      onOpenRemoteConversation,
      remoteCatalogs: [catalogRow("sess_1")],
      remotePeers: [peer],
    });

    expect(view.text()).toContain("Remote conversation");
    // The machine is named by its icon in the row, so the title carries the name.
    expect(view.container.querySelector("[title*='Studio iMac']")).not.toBeNull();

    act(() => view.container.querySelector<HTMLButtonElement>("[title*='Studio iMac']")!.click());
    expect(onOpenRemoteConversation).toHaveBeenCalledTimes(1);
    expect(onOpenRemoteConversation.mock.calls[0]![0]).toMatchObject({
      desktopId: "desktop_a",
      id: "sess_1",
      threadId: "sess_1_thread",
    });
    view.unmount();
  });

  /**
   * A host action must re-read the host, otherwise the list keeps showing the
   * state the user just changed.
   */
  it("re-reads the host after one of its conversations changes", async () => {
    const onRemoteConversationsChanged = vi.fn();
    const view = await mountRail({
      deviceFilter: { kind: "all" },
      onRemoteConversationsChanged,
      remoteCatalogs: [catalogRow("sess_1")],
      remotePeers: [peer],
    });

    act(() => view.actionsButton()!.click());
    act(() => view.item("Pin")!.click());
    await settle();

    expect(pin).toHaveBeenCalledWith("desktop_a", { sessionId: "sess_1", threadId: "sess_1_thread" }, true);
    expect(onRemoteConversationsChanged).toHaveBeenCalledTimes(1);
    view.unmount();
  });

  /** Rename hands off to the shell's dialog, which is not the local one. */
  /**
   * A delete from the row's menu is reported to the shell as a *deletion*.
   *
   * Not just "a host action happened": the shell is what holds the open
   * conversation, and only it can close a column that is drawing the row that
   * just went away. Reported through `onChangedRemote` instead, the catalogue
   * would be re-read and the history would stay on screen — which is how it was
   * reported.
   */
  it("reports a conversation deleted from its own row to the shell", async () => {
    const onRemoteConversationDeleted = vi.fn();
    const view = await mountRail({
      deviceFilter: { kind: "all" },
      onRemoteConversationDeleted,
      remoteCatalogs: [catalogRow("sess_1")],
      remotePeers: [peer],
    });

    act(() => view.actionsButton()!.click());
    act(() => view.item("Delete")!.click());
    await settle();

    expect(remove).toHaveBeenCalledWith("desktop_a", { sessionId: "sess_1", threadId: "sess_1_thread" });
    expect(onRemoteConversationDeleted).toHaveBeenCalledTimes(1);
    expect(onRemoteConversationDeleted.mock.calls[0]![0]).toMatchObject({ desktopId: "desktop_a", id: "sess_1" });
    view.unmount();
  });

  it("offers rename for a remote conversation through the shell", async () => {
    const onRenameRemoteConversation = vi.fn();
    const view = await mountRail({
      deviceFilter: { kind: "all" },
      onRenameRemoteConversation,
      remoteCatalogs: [catalogRow("sess_1")],
      remotePeers: [peer],
    });

    act(() => view.actionsButton()!.click());
    act(() => view.item("Rename")!.click());

    expect(onRenameRemoteConversation).toHaveBeenCalledTimes(1);
    expect(onRenameRemoteConversation.mock.calls[0]![0]).toMatchObject({ desktopId: "desktop_a", id: "sess_1" });
    view.unmount();
  });

  /**
   * With a host chosen, the list shows that machine's conversations *and* the
   * local ones, ordered together — that is the whole point of the selector.
   */
  it("keeps local and remote conversations in one ordered list", async () => {
    const view = await mountRail({
      deviceFilter: { kind: "all" },
      remoteCatalogs: [catalogRow("sess_remote", { createdAt: 9_000, lastMessageAt: 9_000 })],
      remotePeers: [peer],
      threads: [thread("local_newer", { createdAt: 10_000, title: "Local newer", updatedAt: 10_000 })],
    });

    const titles = [...view.container.querySelectorAll("[title]")]
      .map(node => node.getAttribute("title")!.split(" · ")[0]);
    expect(titles).toContain("Local newer");
    expect(titles).toContain("Remote conversation");
    // The newer conversation first, regardless of which machine it lives on —
    // the start time, which both machines report.
    expect(titles.indexOf("Local newer")).toBeLessThan(titles.indexOf("Remote conversation"));
    view.unmount();
  });

  /** A device filter hides the other machines' rows. */
  it("shows only the chosen machine's conversations when one is filtered", async () => {
    const view = await mountRail({
      deviceFilter: { desktopId: "desktop_a", kind: "device" },
      remoteCatalogs: [catalogRow("sess_a")],
      remotePeers: [peer],
      threads: [thread("local_thread", { title: "Local thread" })],
    });

    expect(view.text()).toContain("Remote conversation");
    expect(view.text()).not.toContain("Local thread");
    view.unmount();
  });
});

/**
 * A local conversation keeps its place in the tree while the list is merged.
 *
 * Merged mode is entered by merely *having* a connected host, so dropping the
 * local tree there silently changed the rail for every user of the client role:
 * a fork rendered as a root (no indent, no toggle) and a parent with thirty
 * forks could not be folded away. The two things the tree gives a reader are
 * asserted here — the toggle exists, and using it hides the forks.
 */
describe("the local tree in merged mode", () => {
  const parent = thread("local_parent", { agentSessionId: "sess_parent", title: "Parent" });
  const child = thread("local_child", {
    agentSessionId: "sess_child",
    parentSessionId: "sess_parent",
    title: "Fork",
  });

  it("offers the fold toggle for a local conversation that has forks", async () => {
    const view = await mountRail({
      remoteCatalogs: [catalogRow("sess_a")],
      remotePeers: [peer],
      threads: [parent, child],
    });

    expect(view.container.querySelector("button[aria-label='Expand child conversations of Parent']")).not.toBeNull();
    view.unmount();
  });

  it("hides a collapsed conversation's forks", async () => {
    const view = await mountRail({
      remoteCatalogs: [catalogRow("sess_a")],
      remotePeers: [peer],
      threads: [parent, child],
    });

    // Collapsed by default, exactly as the local list is.
    expect(view.text()).not.toContain("Fork");

    const toggle = view.container.querySelector<HTMLButtonElement>("button[aria-label='Expand child conversations of Parent']")!;
    act(() => toggle.click());

    // Expanding reveals it without disturbing the remote row.
    expect(view.text()).toContain("Fork");
    expect(view.text()).toContain("Remote conversation");
    view.unmount();
  });

  /**
   * A fork goes where its *conversation* went, not where its own fields point: the
   * forks of a pinned conversation are pinned rows.
   *
   * The merged list draws the pinned section and the workspace groups apart, and
   * a fork carries its own pin and workspace — so a fork of a pinned conversation
   * was drawn under the workspace heading, hundreds of pixels from the `+` the
   * user had just pressed, while the same list filtered to this machine alone
   * showed it directly underneath. A fork is a conversation's, so it takes that
   * conversation's section.
   */
  it("keeps a local fork beside its conversation when that one is pinned", async () => {
    const view = await mountRail({
      remoteCatalogs: [catalogRow("sess_a")],
      remotePeers: [peer],
      threads: [
        thread("local_parent", {
          agentSessionId: "sess_parent",
          lastMessageAt: 5_000,
          mode: "workspace",
          pinned: true,
          title: "Pinned parent",
          workspaceId: "ws_1",
        }),
        thread("local_fork", {
          agentSessionId: "sess_child",
          lastMessageAt: 8_000,
          mode: "workspace",
          parentSessionId: "sess_parent",
          title: "Fork of pinned",
          workspaceId: "ws_1",
        }),
        // A newer conversation in the same workspace, so a fork ranked by its own
        // time lands below *this* rather than under the row it came from.
        thread("local_other", {
          agentSessionId: "sess_other",
          lastMessageAt: 9_000,
          mode: "workspace",
          title: "Other in the workspace",
          workspaceId: "ws_1",
        }),
      ],
      workspaces: [storedWorkspace("ws_1", "Charts")],
    });

    act(() => view.container
      .querySelector<HTMLButtonElement>("button[aria-label='Expand child conversations of Pinned parent']")!
      .click());

    expect([...view.container.querySelectorAll<HTMLElement>("div[class*='group/thread']")]
      .map(row => ({ indent: row.style.paddingLeft, title: (row.textContent ?? "").trim() })))
      .toEqual([
        { indent: "16px", title: "Pinned parent" },
        { indent: "36px", title: "Fork of pinned" },
        { indent: "28px", title: "Other in the workspace" },
        { indent: "16px", title: "Remote conversation💻" },
      ]);
    view.unmount();
  });
});

/**
 * A conversation on another machine has the tree its host reports: the host
 * sends the session each one was forked from, so the merged list draws the same
 * indent and the same `+`/`−` toggle the local rows have. Asserted through the
 * rail because the tree, the fold state and the row filter are three parts that
 * have to agree — a toggle that folds nothing is the bug this is here for.
 */
describe("the remote tree in merged mode", () => {
  const hostWorkspace = {
    desktopId: "desktop_a",
    sessions: [
      {
        lastMessageAt: 5_000,
        sessionId: "sess_parent",
        threadId: "th_parent",
        title: "Host parent",
        mode: "workspace",
        workspaceId: "ws_1",
        workspaceName: "图表脚本",
      },
      {
        lastMessageAt: 4_000,
        parentSessionId: "sess_parent",
        sessionId: "sess_fork",
        threadId: "th_fork",
        title: "Host fork",
        mode: "workspace",
        workspaceId: "ws_1",
        workspaceName: "图表脚本",
      },
    ],
  } as never;

  it("offers the fold toggle for a remote conversation that has forks", async () => {
    const view = await mountRail({ remoteCatalogs: [hostWorkspace], remotePeers: [peer] });

    expect(view.container.querySelector("button[aria-label='Expand child conversations of Host parent']")).not.toBeNull();
    // A conversation with nothing forked from it has no toggle, as locally.
    expect(view.container.querySelector("button[aria-label='Expand child conversations of Host fork']")).toBeNull();
    view.unmount();
  });

  it("hides a collapsed remote conversation's forks, and reveals them", async () => {
    const view = await mountRail({ remoteCatalogs: [hostWorkspace], remotePeers: [peer] });

    // Collapsed by default, exactly as the local list is.
    expect(view.text()).not.toContain("Host fork");

    const toggle = view.container.querySelector<HTMLButtonElement>("button[aria-label='Expand child conversations of Host parent']")!;
    act(() => toggle.click());

    expect(view.text()).toContain("Host fork");
    view.unmount();
  });

  /**
   * The fork is drawn *under* the conversation it came from — the row that
   * follows it in the list, not a place of its own by time.
   *
   * A fork's last message is newer than its parent's by construction, so ranking
   * rows by recency put it *above* the conversation it came from: pressing the
   * parent's `+` appeared to do nothing, because the row it revealed had been
   * there all along, one line higher. The order is the assertion; the indent is
   * how a reader sees that the two belong together.
   */
  it("draws a fork under its conversation, not where its own time puts it", async () => {
    const view = await mountRail({
      remoteCatalogs: [{
        desktopId: "desktop_a",
        sessions: [
          { lastMessageAt: 9_000, parentSessionId: "sess_parent", sessionId: "sess_fork", threadId: "th_fork", title: "Host fork" },
          { lastMessageAt: 1_000, sessionId: "sess_parent", threadId: "th_parent", title: "Host parent" },
        ],
      } as never],
      remotePeers: [peer],
    });

    const order = () => [...view.container.querySelectorAll<HTMLElement>("div[class*='group/thread']")]
      .map(row => ({ indent: row.style.paddingLeft, title: (row.textContent ?? "").trim() }));
    // Folded: only the parent, with its toggle.
    expect(order().map(row => row.title)).toEqual(["Host parent💻"]);

    act(() => view.container
      .querySelector<HTMLButtonElement>("button[aria-label='Expand child conversations of Host parent']")!
      .click());

    expect(order()).toEqual([
      { indent: "16px", title: "Host parent💻" },
      { indent: "36px", title: "Host fork💻" },
    ]);
    view.unmount();
  });
});

/**
 * The merged list's chrome is the rail's own, and its controls stay wired to the
 * rail's state: folding a section, creating a workspace or a chat, and re-reading
 * a host after one of its rows was acted on.
 *
 * Asserted through the rail because that is where those handlers live — a section
 * header that renders but does not fold is a control that does nothing.
 */
describe("the merged list's controls", () => {
  it("folds the workspace and chat sections from their headers", async () => {
    const view = await mountRail({ remoteCatalogs: [catalogRow("sess_a")] });

    await act(async () => view.container.querySelector<HTMLButtonElement>("button[aria-label='Collapse workspace list']")!.click());
    expect(view.container.querySelector("button[aria-label='Expand workspace list']")).not.toBeNull();

    await act(async () => view.container.querySelector<HTMLButtonElement>("button[aria-label='Collapse chat list']")!.click());
    expect(view.container.querySelector("button[aria-label='Expand chat list']")).not.toBeNull();
    view.unmount();
  });

  it("offers this machine's creation controls, and hands them the rail's handlers", async () => {
    const onNewWorkspace = vi.fn();
    const onNewChat = vi.fn();
    const view = await mountRail({
      onNewChat,
      onNewWorkspace,
      remoteCatalogs: [catalogRow("sess_a")],
    });

    await act(async () => view.container.querySelector<HTMLButtonElement>("button[aria-label='New workspace']")!.click());
    expect(onNewWorkspace).toHaveBeenCalledTimes(1);

    await act(async () => view.container.querySelector<HTMLButtonElement>("button[aria-label='New chat']")!.click());
    expect(onNewChat).toHaveBeenCalledTimes(1);
    view.unmount();
  });

  it("re-reads a host when one of its rows changed", async () => {
    const onRemoteConversationsChanged = vi.fn();
    const view = await mountRail({
      onRemoteConversationsChanged,
      remoteCatalogs: [catalogRow("sess_a")],
    });

    pin.mockResolvedValueOnce(undefined);
    await act(async () => view.actionsButton()!.click());
    await act(async () => view.item("Pin")!.click());
    await settle();

    expect(pin).toHaveBeenCalled();
    // The list the user is looking at is the host's, so it has to be read again
    // — nothing local would show what was just changed there.
    expect(onRemoteConversationsChanged).toHaveBeenCalled();
    view.unmount();
  });
});

/**
 * With a host in the list, this machine's workspace heading is still the rail's
 * own — with its folder, its menu and its fold control.
 *
 * The merged list asks the rail for that heading rather than drawing its own, so
 * one workspace cannot be folded in one list and open in the other.
 */
it("renders this machine's workspace heading through the merged list", async () => {
  const view = await mountRail({
    remoteCatalogs: [catalogRow("sess_a")],
    threads: [thread("th_a", { mode: "workspace", workspaceId: "ws_1" })],
    workspaces: [{ id: "ws_1", name: "future-os", path: "/tmp/ws", pinned: false } as never],
  });

  // The workspace heading, and its fold control, are in the merged list.
  expect(view.container.textContent).toContain("future-os");
  const collapse = view.container.querySelector<HTMLButtonElement>("button[aria-label='Collapse workspace']");
  expect(collapse).not.toBeNull();
  await act(async () => collapse!.click());
  expect(view.container.querySelector("button[aria-label='Expand workspace']")).not.toBeNull();
  view.unmount();
});

/**
 * The rail's own wiring of the merged list's two new controls.
 *
 * The list renders them and the shell owns their handlers; what is asserted here
 * is the arrow in between — a control whose handler is never reached is a control
 * that does nothing.
 */
describe("the merged list's workspace controls", () => {
  it("starts a conversation in a host's workspace, and marks chats for selection", async () => {
    const onNewChatOnRemote = vi.fn();
    const view = await mountRail({
      onNewChatOnRemote,
      remoteCatalogs: [catalogRow("sess_a", { mode: "workspace", workspaceId: "ws_9", workspaceName: "图表脚本" })],
    });

    // The heading's own "+" names the host and its workspace.
    const add = view.container.querySelector<HTMLButtonElement>("button[aria-label^='New chat in']")!;
    await act(async () => add.click());
    expect(onNewChatOnRemote).toHaveBeenCalledWith("desktop_a", "ws_9");

    // The chat section's menu enters selection mode, which is a local operation.
    await act(async () => view.container.querySelector<HTMLButtonElement>("button[aria-label='Chat section actions']")!.click());
    await act(async () => {
      [...view.container.querySelectorAll("button")].find(node => node.textContent === "Select chats")!.click();
    });
    expect(view.container.querySelector("[data-child='selection-toolbar'], [role='toolbar']")).not.toBeNull();
    view.unmount();
  });
});

/**
 * A batch can hold a host's conversations.
 *
 * One selection over every row the list shows: a host's row carries the same
 * checkbox a local one does, the toolbar counts it, and Delete hands it to the
 * shell — which is where the host command belongs, because a remote row's
 * deletion addresses a machine this component knows nothing about.
 */
it("collects a host's conversations into a batch and hands them to the shell", async () => {
  const onDeleteRemoteConversations = vi.fn();
  const onBatchDeleteThreads = vi.fn();
  const view = await mountRail({
    deviceFilter: { kind: "all" },
    onBatchDeleteThreads,
    onDeleteRemoteConversations,
    remoteCatalogs: [catalogRow("sess_1")],
    remotePeers: [peer],
  });

  // Entered from the chat section, which now lists a host's chats too.
  await act(async () => view.container.querySelector<HTMLButtonElement>("button[aria-label='Chat section actions']")!.click());
  await act(async () => {
    [...view.container.querySelectorAll("button")].find(node => node.textContent === "Select chats")!.click();
  });

  const checkbox = view.container.querySelector<HTMLInputElement>("input[type='checkbox']")!;
  await act(async () => checkbox.click());
  expect(view.text()).toContain("1 selected");
  // `selectedCount === totalCount` by now, so the toolbar's own box is the
  // *deselect* control; the Delete button is the one under test.
  await act(async () => view.container.querySelector<HTMLButtonElement>("button[aria-label='Delete']")!.click());
  // The host's row goes to the host, and never to this machine's batch.
  expect(onDeleteRemoteConversations).toHaveBeenCalledTimes(1);
  expect(onDeleteRemoteConversations.mock.calls[0]![0].map((row: { id: string }) => row.id)).toEqual(["sess_1"]);
  expect(onBatchDeleteThreads).not.toHaveBeenCalled();
  view.unmount();
});

/**
 * The local heading's "select chats" item, on a workspace that lives elsewhere.
 *
 * The item was missing from a remote workspace's menu, so the only way to batch
 * another machine's conversations was to go to that machine.
 */
it("offers select-chats on a host's workspace heading, scoping the batch to it", async () => {
  const view = await mountRail({
    remoteCatalogs: [catalogRow("sess_ws", { mode: "workspace", workspaceId: "ws_9", workspaceName: "图表脚本" })],
    remotePeers: [peer],
  });

  await act(async () => view.container.querySelector<HTMLButtonElement>("button[aria-label^='Workspace actions']")!.click());
  await act(async () => {
    [...view.container.querySelectorAll("button")].find(node => node.textContent === "Select chats")!.click();
  });
  expect(view.container.querySelector("[role='toolbar']")).not.toBeNull();

  // The scope is that workspace: its own row is selectable.
  const checkbox = view.container.querySelector<HTMLInputElement>("input[type='checkbox']")!;
  await act(async () => checkbox.click());
  expect(view.text()).toContain("1 selected");
  view.unmount();
});

/**
 * A heading's controls are revealed by *that heading*, not by the list around it.
 *
 * The list area is itself an unnamed `group` — that is what reveals the floating
 * scrollbar on hover — so a bare `group-hover:` on a heading's controls matched
 * the whole area: measured in a real browser, the pointer resting on a
 * conversation row showed the actions menu and the "new conversation" plus of
 * every heading at once ("all the headings look hovered"), and moving the
 * pointer between headings never changed which of them had controls.
 *
 * Asserted on classes because jsdom applies no styles: what decides this is which
 * named group a `group-hover:` modifier points at, and an unnamed one in here is
 * the bug itself — nothing in this list may be revealed by the area's own group.
 */
it("reveals a heading's controls with that heading, not with the list it sits in", async () => {
  const view = await mountRail({
    deviceFilter: { kind: "all" },
    remoteCatalogs: [catalogRow("sess_ws", {
      mode: "workspace",
      workspaceId: "ws_9",
      workspaceName: "Remote project",
    })],
    remotePeers: [peer],
    threads: [thread("t_1", { mode: "workspace", workspaceId: "ws_1" })],
    workspaces: [storedWorkspace("ws_1", "Local project")],
  });
  const content = view.container.querySelector<HTMLElement>("[data-activity-rail-content='true']")!;
  // Both headings are drawn, so the rule is asserted on both kinds.
  expect(content.querySelector("[data-testid='local-group-heading']")).not.toBeNull();
  expect(content.querySelector("[data-testid='merged-group-heading']")).not.toBeNull();

  // Every control the list keeps hidden until a heading is hovered: one actions
  // menu and one "new conversation" plus on each heading.
  const revealControls = [...content.querySelectorAll<HTMLButtonElement>("button[class*='opacity-0']")];
  expect(revealControls).toHaveLength(4);
  for (const control of revealControls) {
    expect(control.className).toContain("group-hover/workspace:opacity-100");
    // The bug, in one line: a bare `group-hover:` is revealed by *any* ancestor
    // `group`, and the list area wrapping this content is one.
    expect(control.className).not.toContain("group-hover:");
  }
  // And nothing inside the list may key off the area's own group at all.
  expect(content.querySelectorAll("[class*='group-hover:']")).toHaveLength(0);
  view.unmount();
});

describe("the rail's remote row controls", () => {
  /**
   * A forked remote conversation folds and unfolds like a local one.
   *
   * The fold state is the rail's own (a set of row keys), so the second click is
   * what proves the toggle is a toggle: the first expands, the second has to
   * remove the key again — a control that only ever adds leaves the reader unable
   * to close what they opened.
   */
  it("folds a remote fork open and closed", async () => {
    const view = await mountRail({
      deviceFilter: { kind: "all" },
      remoteCatalogs: [{
        desktopId: "desktop_a",
        sessions: [
          { lastMessageAt: 5_000, sessionId: "sess_parent", threadId: "parent_thread", title: "Parent conversation" },
          { lastMessageAt: 4_000, parentSessionId: "sess_parent", sessionId: "sess_child", threadId: "child_thread", title: "Child conversation" },
        ],
      } as never],
      remotePeers: [peer],
    });

    // The child is hidden until the parent is expanded.
    expect(view.text()).toContain("Parent conversation");
    expect(view.text()).not.toContain("Child conversation");

    const expand = () => view.container.querySelector<HTMLButtonElement>(
      "button[aria-label='Expand child conversations of Parent conversation']",
    );
    act(() => expand()!.click());
    expect(view.text()).toContain("Child conversation");

    const collapse = () => view.container.querySelector<HTMLButtonElement>(
      "button[aria-label='Collapse child conversations of Parent conversation']",
    );
    act(() => collapse()!.click());
    expect(view.text()).not.toContain("Child conversation");
    // And it offers the way back in, rather than leaving the fork shut for good.
    expect(expand()).not.toBeNull();
    view.unmount();
  });

  /**
   * Renaming a workspace on another machine is the shell's dialog, so the rail's
   * job is to name *which* workspace — the machine and the workspace id the host
   * addresses it by, plus the name the dialog opens with. Handing over only the
   * id would leave the dialog blank; handing over the name alone would leave the
   * host unaddressable.
   */
  it("hands the shell the host, the id and the current name when a remote workspace is renamed", async () => {
    const onRenameRemoteWorkspace = vi.fn();
    const view = await mountRail({
      deviceFilter: { kind: "all" },
      onRenameRemoteWorkspace,
      remoteCatalogs: [catalogRow("sess_ws", {
        mode: "workspace",
        sessionId: "sess_ws",
        threadId: "ws_thread",
        title: "A conversation in the workspace",
        workspaceId: "ws_remote",
        workspaceName: "Docs",
      })],
      remotePeers: [peer],
    });

    act(() => view.container
      .querySelector<HTMLButtonElement>("button[aria-label='Workspace actions for Docs']")!
      .click());
    const rename = [...view.container.querySelectorAll<HTMLButtonElement>("[role='menuitem']")]
      .find(node => node.textContent === "Rename");
    expect(rename).not.toBeUndefined();
    act(() => rename!.click());

    expect(onRenameRemoteWorkspace).toHaveBeenCalledTimes(1);
    expect(onRenameRemoteWorkspace.mock.calls[0]![0]).toEqual({
      desktopId: "desktop_a",
      name: "Docs",
      workspaceId: "ws_remote",
    });
    view.unmount();
  });

  /** The device picker's own way out to the machines screen is the rail's to wire. */
  it("opens the desktops screen from the device picker", async () => {
    const onManageDesktops = vi.fn();
    const view = await mountRail({
      deviceFilter: { kind: "all" },
      onChangeDeviceFilter: vi.fn(),
      onManageDesktops,
      remoteCatalogs: [catalogRow("sess_1")],
      remotePeers: [peer],
    });

    act(() => view.container
      .querySelector<HTMLButtonElement>("button[aria-haspopup='listbox']")!
      .dispatchEvent(new MouseEvent("click", { bubbles: true })));
    const manage = [...view.container.querySelectorAll<HTMLButtonElement>("button")]
      .find(node => node.textContent === "Manage desktops…");
    expect(manage).not.toBeUndefined();
    act(() => manage!.click());

    expect(onManageDesktops).toHaveBeenCalledTimes(1);
    view.unmount();
  });
});

/**
 * The device picker and the search share one row above the list.
 *
 * The picker is drawn `compact` — glyph, name, chevron in a quarter of the row
 * instead of a band of its own — and the search is a single magnifier that opens
 * into a field in the same row. So what this checks is that the row holds exactly
 * those two things, before and after the magnifier is clicked: a second row for
 * either of them is the regression.
 */
describe("the search and the device picker", () => {
  it("shares one row between the picker and the search, closed or open", async () => {
    const view = await mountRail({
      deviceFilter: { kind: "all" },
      onChangeDeviceFilter: vi.fn(),
      remotePeers: [peer],
    });
    const picker = () => view.container.querySelector("button[aria-label='All devices']");
    const glyph = () => view.container.querySelector<HTMLButtonElement>("button[aria-label='Search conversations']");
    const field = () => view.container.querySelector("input[aria-label='Search conversations']");
    const tools = () => view.container.querySelector<HTMLElement>("[data-testid='rail-list-tools']")!;

    expect(picker()).not.toBeNull();
    // A glyph, not a field — the rail keeps the row narrow until it is asked.
    expect(glyph()).not.toBeNull();
    expect(field()).toBeNull();
    expect(tools().contains(picker())).toBe(true);
    expect(tools().contains(glyph())).toBe(true);
    // The picker is the compact one — it kept its name, it gave up the band.
    expect(picker()!.className).toContain("max-w-30");

    act(() => glyph()!.click());
    await flushAsync();
    expect(field()).not.toBeNull();
    expect(tools().contains(field())).toBe(true);
    // The picker stays where it was: opening the field does not push it out.
    expect(tools().contains(picker())).toBe(true);

    view.unmount();
  });
});

/**
 * The device filter from the keyboard.
 *
 * The picker one row above the list is a pointer target, so the keyboard's way
 * through the machines is the ring the picker draws them in: all machines, this
 * one, each paired one, then all machines again.
 */
describe("the device filter from the keyboard", () => {
  let uninstallShortcuts: () => void;
  beforeEach(() => {
    resetAllCommandBindings();
    uninstallShortcuts = installShortcuts();
  });
  afterEach(() => {
    uninstallShortcuts();
    resetAllCommandBindings();
  });

  const press = (init: KeyboardEventInit) => act(() => {
    window.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, cancelable: true, ...init }));
  });

  /** One press from `filter`, with two machines paired. */
  async function stepFrom(filter: ComponentProps<typeof ActivityRail>["deviceFilter"]) {
    const onChangeDeviceFilter = vi.fn();
    const view = await mountRail({
      deviceFilter: filter,
      onChangeDeviceFilter,
      remotePeers: [peer, { ...peer, desktopId: "desktop_b", name: "Second machine" }],
    });
    press({ altKey: false, code: "KeyD", ctrlKey: true, key: "d", shiftKey: true });
    const next = onChangeDeviceFilter.mock.calls[0]?.[0];
    view.unmount();
    return next;
  }

  it("walks all machines, this one and each paired one, then starts over", async () => {
    expect(await stepFrom({ kind: "all" })).toEqual({ desktopId: null, kind: "device" });
    expect(await stepFrom({ desktopId: null, kind: "device" })).toEqual({ desktopId: "desktop_a", kind: "device" });
    expect(await stepFrom({ desktopId: "desktop_a", kind: "device" })).toEqual({ desktopId: "desktop_b", kind: "device" });
    expect(await stepFrom({ desktopId: "desktop_b", kind: "device" })).toEqual({ kind: "all" });
  });

  it("starts the ring over from a machine this build no longer knows", async () => {
    // A stored filter can name a machine that was unpaired since: the ring cannot
    // place it, so the next press goes back to "all machines" rather than guessing.
    expect(await stepFrom({ desktopId: "desktop_gone", kind: "device" })).toEqual({ kind: "all" });
  });

  it("leaves the key unclaimed when nothing else is paired", async () => {
    const onChangeDeviceFilter = vi.fn();
    const view = await mountRail({ onChangeDeviceFilter, remotePeers: [] });
    const event = new KeyboardEvent("keydown", { bubbles: true, cancelable: true, code: "KeyD", ctrlKey: true, key: "d", shiftKey: true });
    act(() => window.dispatchEvent(event));
    expect(onChangeDeviceFilter).not.toHaveBeenCalled();
    // Nothing to switch to, so the press is not consumed either.
    expect(event.defaultPrevented).toBe(false);
    view.unmount();
  });
});
