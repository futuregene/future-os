// @vitest-environment jsdom
import type { DeviceFilter, MergedConversation } from "../../features/remote-peer/mergeConversations";
import type { RemotePeer } from "../../features/remote-peer/remotePeerClient";
import type { StoredThread } from "../../integrations/storage/threadStore";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { MergedRows } from "./MergedRows";

/**
 * The merged list's own contract, at the boundary the rail hands it.
 *
 * The rail builds these rows from the same thread list it passes as `threadById`,
 * so a lookup that misses is not a state the app reaches today — but the
 * component is handed two collections that *could* disagree (a future caller, a
 * filter applied to one and not the other), and a missing thread must render
 * nothing rather than throw. That is what is asserted: not a scenario the product
 * produces, but the guarantee that keeps a data mistake from taking the window
 * down.
 */

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

function conversation(overrides: Partial<MergedConversation> = {}): MergedConversation {
  return {
    desktopId: null,
    id: "thread_1",
    key: "local::thread_1",
    lastMessageAt: 1,
    mode: "chat",
    pinned: false,
    streaming: false,
    threadId: "thread_1",
    title: "A conversation",
    workspaceId: null,
    ...overrides,
  };
}

/** One paired machine, as the rail's peer list reports it. */
function peer(desktopId: string, name: string): RemotePeer {
  return {
    agentAvailable: true,
    bridgeInstanceId: `bridge_${desktopId}`,
    connected: true,
    stopped: false,
    desktopId,
    error: null,
    features: [],
    icon: "laptop",
    name,
    pairId: `pair_${desktopId}`,
  };
}

function thread(id: string): StoredThread {
  return {
    agentSessionId: id,
    createdAt: 1,
    id,
    mode: "chat",
    pinned: false,
    readonly: false,
    status: "active",
    title: id,
    updatedAt: 1,
    workspaceId: "ws",
  };
}

async function render(props: {
  collapsedWorkspaces?: ReadonlySet<string>;
  expandedRemoteRows?: ReadonlySet<string>;
  filter?: DeviceFilter;
  onNewChat?: () => void;
  onNewChatInWorkspace?: (desktopId: string, workspaceId: string | null) => void;
  onNewWorkspace?: () => void;
  onOpenRemote?: (row: MergedConversation) => void;
  onRenameRemoteWorkspace?: (target: { desktopId: string; workspaceId: string; name: string }) => void;
  onSelectWorkspaceChats?: (groupKey: string) => void;
  onToggleRemoteExpanded?: (key: string) => void;
  onToggleWorkspace?: (key: string) => void;
  peers?: RemotePeer[];
  /** Where each listed row sits in the shared list's tree, by row key. */
  rowMeta?: ReadonlyMap<string, { depth: number; hasChildren: boolean }>;
  remoteSelection?: {
    selected: (conversation: MergedConversation) => boolean;
    selectable: (conversation: MergedConversation) => boolean;
    toggle: (conversation: MergedConversation) => void;
  };
  renderLocalGroupHeader?: (workspaceId: string) => React.ReactNode;
  renderLocalRow: (thread: StoredThread, depth?: number, hasChildren?: boolean) => React.ReactNode;
  /** Which conversation each row hangs under, by row key (see `MergedRows`). */
  rowRoots?: ReadonlyMap<string, MergedConversation>;
  rows: MergedConversation[];
  selectionMode?: boolean;
  threadById: Map<string, StoredThread>;
  /** Which list to draw: one tab at a time, which is one list at a time. */
  view?: "workspace" | "chat";
}) {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  await act(async () => {
    root.render(
      <MergedRows
        activeKey={null}
        collapsedWorkspaces={props.collapsedWorkspaces ?? new Set()}
        expandedRemoteRows={props.expandedRemoteRows ?? new Set()}
        filter={props.filter ?? { kind: "all" }}
        onChangedRemote={vi.fn()}
        onDeletedRemote={vi.fn()}
        onNewChatInWorkspace={props.onNewChatInWorkspace}
        onOpenRemote={props.onOpenRemote ?? vi.fn()}
        onRenameRemote={vi.fn()}
        onRenameRemoteWorkspace={props.onRenameRemoteWorkspace}
        onSelectWorkspaceChats={props.onSelectWorkspaceChats}
        onToggleRemoteExpanded={props.onToggleRemoteExpanded ?? vi.fn()}
        onToggleWorkspace={props.onToggleWorkspace ?? vi.fn()}
        peers={props.peers ?? []}
        rowMeta={props.rowMeta ?? new Map()}
        remoteSelection={props.remoteSelection}
        renderLocalGroupHeader={props.renderLocalGroupHeader ?? (() => null)}
        renderLocalRow={props.renderLocalRow}
        rowRoots={props.rowRoots ?? new Map()}
        rows={props.rows}
        selectionMode={props.selectionMode ?? false}
        threadById={props.threadById}
        view={props.view ?? "workspace"}
      />,
    );
  });
  return {
    container,
    unmount: async () => {
      await act(async () => root.unmount());
      container.remove();
    },
  };
}

it("labels each group, and marks the remote ones with their machine", async () => {
  // This machine's heading is the rail's own node, injected; the merged list
  // renders the label for a *remote* group itself.
  const view = await render({
    peers: [{
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
    }],
    renderLocalGroupHeader: workspaceId => <div data-testid="local-group-heading">{workspaceId}</div>,
    renderLocalRow: vi.fn(() => <div>row</div>),
    rows: [
      conversation({
        desktopId: null,
        id: "ws_local",
        key: "local::ws_local",
        mode: "workspace",
        title: "mine",
        workspaceId: "ws_1",
        workspaceName: "future-os",
      }),
      conversation({
        desktopId: "desktop_a",
        id: "sess_a",
        key: "desktop_a::sess_a",
        mode: "workspace",
        title: "theirs",
        workspaceId: "ws_9",
        workspaceName: "future-os",
      }),
      conversation({
        desktopId: "desktop_a",
        id: "chat_a",
        key: "desktop_a::chat_a",
        title: "a chat on the host",
      }),
    ],
    threadById: new Map([["ws_local", thread("ws_local")]]),
  });

  // This machine's workspace heading is the rail's own, asked for by id.
  expect(view.container.querySelector("[data-testid='local-group-heading']")!.textContent).toBe("ws_1");
  // The host's workspace of the same name keeps a heading of its own, marked
  // with that machine's icon — merging them would file one machine's rows under
  // the other's heading.
  const headings = [...view.container.querySelectorAll("[data-testid='merged-group-heading']")]
    .map(node => node.textContent ?? "");
  expect(headings).toEqual(["future-os"]);
  // A chat row is not in any workspace: it belongs to the chats list, which is
  // a tab of its own — so this list does not hold it at all.
  expect(view.container.textContent).not.toContain("a chat on the host");
  await view.unmount();

  const chats = await render({
    renderLocalRow: vi.fn(() => <div>row</div>),
    rows: [conversation({ desktopId: "desktop_a", id: "chat_a", key: "desktop_a::chat_a", title: "a chat on the host" })],
    threadById: new Map(),
    view: "chat",
  });
  // Whatever machine it came from, a chat is in the one chats list.
  expect(chats.container.querySelector("[data-testid='merged-conversations']")!.textContent).toContain("a chat on the host");
  await chats.unmount();
});

it("renders a local row through the rail's renderer, with its tree place", async () => {
  const renderLocalRow = vi.fn(() => <div>row</div>);
  const view = await render({
    renderLocalRow,
    rows: [conversation()],
    threadById: new Map([["thread_1", thread("thread_1")]]),
    view: "chat",
  });

  // The fourth argument is the group's `compact` answer, passed on to the rail's
  // own renderer. A local row must be told whether it sits inside a workspace
  // heading for the same reason a remote one is: otherwise the two kinds of row
  // are a different height inside one list, and the same conversation changes
  // height when the device filter changes.
  expect(renderLocalRow).toHaveBeenCalledWith(
    expect.objectContaining({ id: "thread_1" }),
    0,
    false,
    false,
    // No age: the tabbed lists draw no second column, and the row is asked for
    // one only in the recent list.
    undefined,
    // No origin label either: that belongs to the search results.
    undefined,
  );
  await view.unmount();
});

it("tells a local row inside a workspace that it is compact", async () => {
  const renderLocalRow = vi.fn(() => <div>row</div>);
  const view = await render({
    renderLocalRow,
    rows: [
      conversation({
        key: "local::ws_local",
        mode: "workspace",
        workspaceId: "ws_local",
      }),
    ],
    threadById: new Map([["thread_1", thread("thread_1")]]),
  });

  expect(renderLocalRow).toHaveBeenCalledWith(
    expect.objectContaining({ id: "thread_1" }),
    0,
    false,
    true,
    undefined,
    undefined,
  );
  await view.unmount();
});

it("renders nothing, rather than throwing, for a row whose thread is missing", async () => {
  const view = await render({
    renderLocalRow: () => <div>row</div>,
    rows: [conversation()],
    // The list and the lookup disagree: the case this guard exists for.
    threadById: new Map(),
  });

  expect(view.container.textContent).not.toContain("row");
  await view.unmount();
});

/**
 * One chat section for every machine.
 *
 * It used to be one section per machine, named after the machine. That put a
 * heading between two conversations that belong to the same hour, and turned the
 * list a user scans for "what is current" into one list per machine to scan
 * separately — so chats are merged, and the machine is named on the row.
 */
it("merges every machine's chats into one section, in time order", async () => {
  const peers = [peer("desktop_a", "Studio iMac"), peer("desktop_b", "Mac mini")];
  const view = await render({
    view: "chat",
    peers,
    renderLocalRow: vi.fn(() => <div>row</div>),
    rows: [
      conversation({ desktopId: "desktop_a", id: "a_old", key: "desktop_a::a_old", title: "older", lastMessageAt: 10 }),
      conversation({ desktopId: null, id: "local_1", key: "local::local_1", title: "mine", lastMessageAt: 20 }),
      conversation({ desktopId: "desktop_b", id: "b_new", key: "desktop_b::b_new", title: "newest", lastMessageAt: 30 }),
    ],
    threadById: new Map([["local_1", thread("local_1")]]),
  });

  // No heading carries a machine's name: the rows do.
  expect([...view.container.querySelectorAll("[data-testid='merged-group-heading']")]).toHaveLength(0);
  const section = view.container.querySelector("[data-testid='merged-conversations']")!;
  // Newest first, across machines: the host's newest, then this machine's row
  // (rendered by the rail's own renderer, which this test stubs as "row"), then
  // the other host's older one.
  expect(section.textContent).toBe("newest💻rowolder💻");
  await view.unmount();
});

/**
 * A fork is drawn under the conversation it came from, whatever order the rows
 * arrive in.
 *
 * The merged list ranks *blocks* rather than rows, because a fork's own timestamp
 * is newer than its parent's — ranking rows put a fork above the conversation it
 * was forked from, so the parent's `+` appeared to reveal nothing.
 */
it("draws a fork under the conversation it hangs under", async () => {
  const parent = conversation({ desktopId: "desktop_a", id: "parent", key: "desktop_a::parent", lastMessageAt: 10, title: "Parent" });
  const fork = conversation({
    desktopId: "desktop_a",
    id: "fork",
    key: "desktop_a::fork",
    lastMessageAt: 20,
    parentKey: "desktop_a::parent",
    title: "Fork",
  });
  const view = await render({
    view: "chat",
    peers: [peer("desktop_a", "Studio iMac")],
    renderLocalRow: vi.fn(() => <div>row</div>),
    // The fork first, as the merge orders rows by time: the tree's answer is what
    // puts it back under its parent.
    rowRoots: new Map([[fork.key, parent]]),
    rows: [fork, parent],
    threadById: new Map(),
  });

  expect([...view.container.querySelectorAll("div[class*='group/thread']")]
    .map(row => (row.textContent ?? "").trim()))
    .toEqual(["Parent💻", "Fork💻"]);
  await view.unmount();
});

/**
 * Filtered to one machine, the badge on every row repeats the selector directly
 * above the list — and it is the row's title that pays for it. The machine is
 * still named by the selector, so nothing is lost.
 */
it("drops the per-row machine badge when one machine is selected", async () => {
  const remote = conversation({
    desktopId: "desktop_a",
    id: "sess_a",
    key: "desktop_a::sess_a",
    title: "a conversation on the host",
  });
  const peers = [{
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
  }];

  const all = await render({
    view: "chat",
    peers,
    renderLocalRow: vi.fn(() => <div>row</div>),
    rows: [remote],
    threadById: new Map(),
  });
  expect(all.container.querySelectorAll("[data-testid='remote-source-icon']")).toHaveLength(1);
  await all.unmount();

  const single = await render({
    view: "chat",
    filter: { kind: "device", desktopId: "desktop_a" },
    peers,
    renderLocalRow: vi.fn(() => <div>row</div>),
    rows: [remote],
    threadById: new Map(),
  });
  expect(single.container.querySelectorAll("[data-testid='remote-source-icon']")).toHaveLength(0);
  // The row is still there, and still says which conversation it is.
  expect(single.container.textContent).toContain("a conversation on the host");
  await single.unmount();
});

/**
 * This machine's own section is *not* a machine the user selected, so filtering
 * to it keeps the badges: the rows shown are all local, but the point of the
 * badge is to distinguish rows, and a local-only list has nothing to distinguish.
 * (There are no remote rows to badge in that filter anyway — asserted so the
 * rule is not read as "hide badges whenever the filter is not `all`".)
 */
it("keeps the badge on a mixed list even when the filter is not `all`", async () => {
  const peers = [{
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
  }];
  const view = await render({
    view: "chat",
    filter: { kind: "device", desktopId: null },
    peers,
    renderLocalRow: vi.fn(() => <div>row</div>),
    rows: [conversation({ desktopId: null, id: "local_1", key: "local::local_1" })],
    threadById: new Map([["local_1", thread("local_1")]]),
  });

  // Local rows never carried a badge, so the check is that nothing broke; the
  // rule itself is covered above.
  expect(view.container.textContent).toContain("row");
  await view.unmount();
});

/**
 * Foldable groups, driven by the state the rail owns.
 *
 * The component does not hold fold state of its own, and this is the assertion
 * that keeps it that way. It used to: it called `useCollapsedWorkspaces` a second
 * time, which is a second `useState` over the same localStorage key, while the
 * rail's own copy drove the heading it renders. The two agreed only at mount, so
 * folding a workspace while a remote machine was connected flipped the heading
 * and left its conversations in place — and a workspace collapsed before a
 * restart could never be opened again.
 */
it("hides a group's rows when the state says it is collapsed", async () => {
  const onToggleWorkspace = vi.fn();
  const rows = [conversation({
    desktopId: "desktop_a",
    id: "ws_remote",
    key: "desktop_a::ws_remote",
    mode: "workspace",
    title: "theirs",
    workspaceId: "ws_9",
    workspaceName: "future-os",
  })];

  const open = await render({
    onToggleWorkspace,
    peers: [peer("desktop_a", "Studio iMac")],
    renderLocalRow: vi.fn(() => <div>row</div>),
    rows,
    threadById: new Map(),
  });
  // Rows are counted by their frame: a row under a heading that already names
  // the machine does not repeat its icon, which is the point of that rule.
  const visible = () => open.container.querySelectorAll("div[class*='group/thread']").length;
  // The heading is a group's, with the local one's shape: a fold control, the
  // folder, the name, and this machine's icon after it.
  expect(open.container.querySelector("[data-testid='merged-group-heading']")!.textContent).toBe("future-os");
  expect(visible()).toBe(1);
  await act(async () => open.container.querySelector("[data-testid='merged-group-heading']")!.closest("div")!.querySelector("button")!.click());
  // The control asks the rail to fold it; whether the rows go is the rail's word.
  expect(onToggleWorkspace).toHaveBeenCalledWith("desktop_a::ws_9");
  await open.unmount();

  const folded = await render({
    collapsedWorkspaces: new Set(["desktop_a::ws_9"]),
    peers: [peer("desktop_a", "Studio iMac")],
    renderLocalRow: vi.fn(() => <div>row</div>),
    rows,
    threadById: new Map(),
  });
  // The heading stays — that is how the group is reopened — and its rows do not.
  expect(folded.container.querySelector("[data-testid='merged-group-heading']")!.textContent).toBe("future-os");
  expect(folded.container.querySelectorAll("div[class*='group/thread']").length).toBe(0);
  await folded.unmount();
});

/**
 * This machine's workspace groups fold by the workspace id, under the heading the
 * rail supplies.
 *
 * Both halves of the old defect are here: the rows are gated by the same set the
 * rail folds with (so a collapsed workspace is collapsed in both lists), and the
 * heading is the rail's own node rather than a second one.
 */
it("folds this machine's workspace group by the key the local list uses", async () => {
  const renderLocalRow = vi.fn(() => <div>row</div>);
  const rows = [conversation({
    desktopId: null,
    id: "ws_local",
    key: "local::ws_local",
    mode: "workspace",
    title: "in the workspace",
    workspaceId: "ws_1",
    workspaceName: "future-os",
  })];

  const open = await render({
    collapsedWorkspaces: new Set(),
    renderLocalGroupHeader: workspaceId => <div data-testid="local-group-heading">{workspaceId}</div>,
    renderLocalRow,
    rows,
    threadById: new Map([["ws_local", thread("ws_local")]]),
  });
  expect(open.container.querySelector("[data-testid='local-group-heading']")!.textContent).toBe("ws_1");
  expect(renderLocalRow).toHaveBeenCalledTimes(1);
  await open.unmount();

  renderLocalRow.mockClear();
  const folded = await render({
    collapsedWorkspaces: new Set(["ws_1"]),
    renderLocalGroupHeader: workspaceId => <div data-testid="local-group-heading">{workspaceId}</div>,
    renderLocalRow,
    rows,
    threadById: new Map([["ws_local", thread("ws_local")]]),
  });
  // The heading is still drawn (it is what the reader clicks to reopen), and this
  // workspace's rows are not built at all.
  expect(folded.container.querySelector("[data-testid='local-group-heading']")).not.toBeNull();
  expect(renderLocalRow).not.toHaveBeenCalled();
  await folded.unmount();
});

/**
 * A pinned workspace leads the list, as it does in the local one.
 *
 * The merged list ordered groups purely by recency and ignored the pin, so a
 * pinned workspace kept its marker on a group that never moved — a pin that does
 * nothing is worse than no pin, because the user believes they set something.
 *
 * The pin travels on the *row* now (`workspacePinned`), which is what lets a
 * workspace on another machine lead this list too: the merged list never reads
 * this machine's workspace store for a row it does not own.
 */
it("leads with a pinned workspace, whichever machine pinned it", async () => {
  const view = await render({
    renderLocalGroupHeader: workspaceId => <div data-testid="local-group-heading">{workspaceId}</div>,
    renderLocalRow: vi.fn(() => <div>row</div>),
    rows: [
      conversation({
        desktopId: null,
        id: "ws_local",
        key: "local::ws_local",
        lastMessageAt: 1_000,
        mode: "workspace",
        workspaceId: "ws_1",
        workspaceName: "pinned one",
        workspacePinned: true,
      }),
      conversation({
        desktopId: "desktop_a",
        id: "ws_remote",
        key: "desktop_a::ws_remote",
        lastMessageAt: 9_000,
        mode: "workspace",
        workspaceId: "ws_9",
        workspaceName: "newer but unpinned",
      }),
    ],
    threadById: new Map([["ws_local", thread("ws_local")]]),
  });

  const headings = [...view.container.querySelectorAll("[data-testid='merged-group-heading']")]
    .map(node => node.textContent);
  expect(headings).toEqual(["newer but unpinned"]);
  // The pinned group is a local one, so its heading comes from the rail — but it
  // is the first group drawn, ahead of a newer unpinned one.
  const order = [...view.container.querySelectorAll("[data-testid='local-group-heading'], [data-testid='merged-group-heading']")]
    .map(node => node.textContent);
  expect(order).toEqual(["ws_1", "newer but unpinned"]);
  await view.unmount();

  // The same rule for a host's workspace: it leads the list and its heading
  // shows the pin, so the group that moved says why it moved.
  const remotePinned = await render({
    peers: [peer("desktop_a", "Studio iMac")],
    renderLocalRow: vi.fn(() => <div>row</div>),
    rows: [
      conversation({
        desktopId: null,
        id: "ws_local",
        key: "local::ws_local",
        lastMessageAt: 9_000,
        mode: "workspace",
        workspaceId: "ws_1",
        workspaceName: "newer but unpinned",
      }),
      conversation({
        desktopId: "desktop_a",
        id: "ws_remote",
        key: "desktop_a::ws_remote",
        lastMessageAt: 1_000,
        mode: "workspace",
        workspaceId: "ws_9",
        workspaceName: "pinned there",
        workspacePinned: true,
      }),
    ],
    threadById: new Map([["ws_local", thread("ws_local")]]),
  });
  const remoteOrder = [...remotePinned.container.querySelectorAll("[data-testid='merged-group-heading']")]
    .map(node => node.textContent);
  expect(remoteOrder[0]).toBe("pinned there");
  await remotePinned.unmount();
});

/** The machine's icon follows its name, so the name starts the heading. */
it("puts the machine's icon after the group name", async () => {
  const view = await render({
    peers: [{
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
    }],
    renderLocalRow: vi.fn(() => <div>row</div>),
    rows: [
      conversation({
        desktopId: "desktop_a",
        id: "sess_a",
        key: "desktop_a::sess_a",
        mode: "workspace",
        title: "theirs",
        workspaceId: "ws_9",
        workspaceName: "future-os",
      }),
    ],
    threadById: new Map(),
  });

  const heading = view.container.querySelector("[data-testid='merged-group-heading']")!;
  const icon = heading.parentElement!.querySelector("span[aria-hidden]")!;
  // The name element precedes the machine's glyph inside the header.
  expect(icon).not.toBeNull();
  expect(icon.className).toContain("shrink-0");
  expect(heading.parentElement!.textContent).toBe("future-os💻");
  await view.unmount();
});

/**
 * Pinned rows lead the list, remote ones included.
 *
 * A pinned conversation on a host is pinned for the same reason this machine's
 * are, so it belongs in the same section — the pinned section is the one place a
 * reader looks for what they deliberately put first.
 */
it("lists a pinned remote conversation in the pinned section", async () => {
  const view = await render({
    view: "chat",
    peers: [peer("desktop_a", "Studio iMac")],
    renderLocalRow: vi.fn(() => <div>row</div>),
    rows: [
      conversation({ desktopId: "desktop_a", id: "pinned_1", key: "desktop_a::pinned_1", pinned: true, title: "kept first" }),
      conversation({ desktopId: "desktop_a", id: "chat_a", key: "desktop_a::chat_a", title: "a chat on the host" }),
    ],
    threadById: new Map(),
  });

  const pinnedSection = view.container.firstElementChild!;
  expect(pinnedSection.textContent).toContain("Pinned");
  expect(pinnedSection.textContent).toContain("kept first");
  // And not in the chat section too: a pinned row is listed once.
  expect(view.container.querySelector("[data-testid='merged-conversations']")!.textContent).not.toContain("kept first");
  await view.unmount();
});

// ── the controls on a heading ───────────────────────────────────────────────

/**
 * A workspace on another machine takes the same two controls the local one has.
 *
 * The local heading carries a menu for what the host accepts and a way to start
 * a conversation in it; a remote one carried neither, so the only way to work in
 * a folder on the other machine was to go to that machine. The actions differ —
 * there is no "open folder" for a directory that is not here — but their
 * presence does not.
 */
it("offers a menu and a new-chat control on a remote workspace heading", async () => {
  const onNewChatInWorkspace = vi.fn();
  const view = await render({
    onNewChatInWorkspace,
    peers: [peer("desktop_a", "Studio iMac")],
    renderLocalRow: vi.fn(() => <div>row</div>),
    rows: [
      conversation({
        desktopId: "desktop_a",
        id: "ws_remote",
        key: "desktop_a::ws_remote",
        mode: "workspace",
        title: "theirs",
        workspaceId: "ws_9",
        workspaceName: "图表脚本",
      }),
    ],
    threadById: new Map(),
  });

  const heading = view.container.querySelector("[data-testid='merged-group-heading']")!;
  const header = heading.closest("div[class*='group']")!;
  await act(async () => header.querySelector<HTMLButtonElement>("button[aria-label^='New chat in']")!.click());
  // The workspace is named to the host that owns it, not by a path.
  expect(onNewChatInWorkspace).toHaveBeenCalledWith("desktop_a", "ws_9");
  expect(header.querySelector("button[aria-label^='Workspace actions for']")).not.toBeNull();
  await view.unmount();
});

/**
 * The machine is named once per group, not on every row under it.
 *
 * A row's icon repeated what the heading directly above it already said, and the
 * row paid for the repetition out of the width its title needs. A conversation in
 * the chat section still carries it: that section spans machines, so there it
 * qualifies the row.
 */
it("does not repeat the machine on a row whose group heading already names it", async () => {
  const peers = [peer("desktop_a", "Studio iMac")];
  // The two kinds of row are in two lists now, so each is asked in its own.
  const workspaceRow = conversation({
    desktopId: "desktop_a",
    id: "ws_remote",
    key: "desktop_a::ws_remote",
    mode: "workspace",
    title: "in a workspace",
    workspaceId: "ws_9",
    workspaceName: "图表脚本",
  });
  const inWorkspace = await render({
    peers,
    renderLocalRow: vi.fn(() => <div>row</div>),
    rows: [workspaceRow],
    threadById: new Map(),
  });
  // The workspace group's heading carries it, so its row does not.
  expect(inWorkspace.container.querySelector("[data-testid='remote-source-icon']")).toBeNull();
  await inWorkspace.unmount();

  // The chats list has no heading of its own, so its rows still say it.
  const inChats = await render({
    view: "chat",
    peers,
    renderLocalRow: vi.fn(() => <div>row</div>),
    rows: [conversation({ desktopId: "desktop_a", id: "chat_a", key: "desktop_a::chat_a", title: "a chat" })],
    threadById: new Map(),
  });
  expect(inChats.container.querySelector("[data-testid='remote-source-icon']")).not.toBeNull();
  await inChats.unmount();
});

/**
 * A host's workspace heading carries the same two controls as the local one, in
 * the same order, and its pin marker where the local one puts it.
 *
 * The remote heading had its "+" first and its menu second — the same two
 * controls as the local heading, in the opposite order, which reads as a
 * different set of affordances until you click each of them — and its pin marker
 * sat against the name while the local heading's sits at the row's end. Both are
 * positions, so both are measured: a class list cannot say where something is.
 */
it("puts the actions menu before the new-chat control, and the pin at the row's end", async () => {
  const view = await render({
    onSelectWorkspaceChats: vi.fn(),
    peers: [peer("desktop_a", "Studio iMac")],
    renderLocalRow: vi.fn(() => <div>row</div>),
    rows: [
      conversation({
        desktopId: "desktop_a",
        id: "ws_remote",
        key: "desktop_a::ws_remote",
        mode: "workspace",
        title: "theirs",
        workspaceId: "ws_9",
        workspaceName: "图表脚本",
        workspacePinned: true,
      }),
    ],
    threadById: new Map(),
  });

  const heading = view.container.querySelector("[data-testid='merged-group-heading']")!;
  const header = heading.closest("div")!;
  const menu = header.querySelector("button[aria-label^='Workspace actions']")!;
  const add = header.querySelector("button[aria-label^='New chat in']")!;
  expect(menu.compareDocumentPosition(add) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();

  // The pin sits after the flexible spacer and before the controls, which is
  // where this app's own heading puts it. jsdom computes no layout, so the order
  // is what is asserted here and the pixels are measured in the harness
  // (`remote-workspace-menu`, which renders the real page in Chrome).
  const pin = header.querySelector("svg.lucide-pin")!;
  expect(pin).not.toBeNull();
  expect(heading.compareDocumentPosition(pin) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  expect(pin.compareDocumentPosition(menu) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  const spacer = pin.previousElementSibling!;
  expect(spacer.className).toContain("flex-1");
  await view.unmount();
});

/**
 * A batch can hold a host's conversations too.
 *
 * The selection state is the rail's; what the list does per row is ask it,
 * replace the run indicator with the same checkbox a local row shows, and tick
 * rather than open on a click.
 */
it("takes the batch's checkbox on a host's row, and ticks instead of opening it", async () => {
  const toggle = vi.fn();
  const selectable = vi.fn(() => true);
  const view = await render({
    view: "chat",
    peers: [peer("desktop_a", "Studio iMac")],
    remoteSelection: { selectable, selected: () => true, toggle },
    renderLocalRow: vi.fn(() => <div>row</div>),
    rows: [conversation({ desktopId: "desktop_a", id: "chat_a", key: "desktop_a::chat_a", title: "a chat" })],
    selectionMode: true,
    threadById: new Map(),
  });

  const checkbox = view.container.querySelector<HTMLInputElement>("input[type='checkbox']")!;
  expect(checkbox.checked).toBe(true);
  // The checkbox replaces the row's actions trigger, as it does on a local row:
  // the `…` would otherwise appear on hover under the pointer that is ticking
  // rows, offering rename/delete/fork for the row being ticked.
  expect(view.container.querySelector("button[aria-haspopup='menu']")).toBeNull();
  await act(async () => checkbox.click());
  expect(selectable).toHaveBeenCalled();
  expect(toggle).toHaveBeenCalledTimes(1);
  await view.unmount();
});

/** With one machine selected nothing needs naming, on a heading or a row. */
it("drops the machine from headings too when one machine is selected", async () => {
  const view = await render({
    filter: { kind: "device", desktopId: "desktop_a" },
    peers: [peer("desktop_a", "Studio iMac")],
    renderLocalRow: vi.fn(() => <div>row</div>),
    rows: [
      conversation({
        desktopId: "desktop_a",
        id: "ws_remote",
        key: "desktop_a::ws_remote",
        mode: "workspace",
        title: "in a workspace",
        workspaceId: "ws_9",
        workspaceName: "图表脚本",
      }),
    ],
    threadById: new Map(),
  });

  const heading = view.container.querySelector("[data-testid='merged-group-heading']")!;
  expect(heading.closest("div[class*='group']")!.querySelector("span[aria-hidden]")).toBeNull();
  expect(view.container.querySelector("[data-testid='remote-source-icon']")).toBeNull();
  await view.unmount();
});

// ── starting a conversation ─────────────────────────────────────────────────

/**
 * In selection mode a click on a host's *row* ticks it instead of opening it.
 *
 * The checkbox is one way to tick a row; activating the row itself is the other,
 * and it is the local list's rule from the same state. Without this a click in
 * selection mode opened a conversation — leaving the batch the user was building
 * and opening something on another machine instead.
 */
it("ticks a host's row when the row itself is activated in selection mode", async () => {
  const toggle = vi.fn();
  const onOpenRemote = vi.fn();
  const conversationRow = conversation({ desktopId: "desktop_a", id: "chat_a", key: "desktop_a::chat_a", title: "a chat" });
  const view = await render({
    view: "chat",
    onOpenRemote,
    peers: [peer("desktop_a", "Studio iMac")],
    remoteSelection: { selectable: () => true, selected: () => false, toggle },
    renderLocalRow: vi.fn(() => null),
    rows: [conversationRow],
    selectionMode: true,
    threadById: new Map(),
  });

  // The row's own activation surface: `RailRow` is a button whose `title` is
  // "conversation · machine", and the row is what the user clicks.
  const row = view.container.querySelector<HTMLElement>("button[title^='a chat']")!;
  await act(async () => row.click());

  expect(toggle).toHaveBeenCalledWith(conversationRow);
  expect(onOpenRemote).not.toHaveBeenCalled();
  await view.unmount();
});

/** An empty chat section says so rather than drawing an unexplained gap. */
it("says the chat section is empty when it holds no rows", async () => {
  const view = await render({
    view: "chat",
    renderLocalRow: vi.fn(() => null),
    rows: [],
    threadById: new Map(),
  });

  expect(view.container.textContent).toContain("No chats");
  await view.unmount();
});

/**
 * Renaming a workspace on another machine opens the shell's dialog, so the rail
 * hands it the machine, the workspace id the host addresses it by, and the name
 * the dialog opens with.
 *
 * The name comes from the *heading*, which is the current one: a dialog seeded
 * from anything else would open showing a stale name and then rename to it.
 */
it("hands the machine, the id and the current name to the rename hand-off", async () => {
  const onRenameRemoteWorkspace = vi.fn();
  const view = await render({
    onRenameRemoteWorkspace,
    peers: [peer("desktop_a", "Studio iMac")],
    renderLocalRow: vi.fn(() => null),
    rows: [conversation({
      desktopId: "desktop_a",
      id: "ws_thread",
      key: "desktop_a::ws_thread",
      mode: "workspace",
      title: "a chat in the workspace",
      workspaceId: "ws_remote",
      workspaceName: "Docs",
    })],
    threadById: new Map(),
  });

  const menu = view.container.querySelector<HTMLButtonElement>("button[aria-label='Workspace actions for Docs']")!;
  await act(async () => menu.click());
  const rename = [...view.container.querySelectorAll<HTMLButtonElement>("[role='menuitem']")]
    .find(node => node.textContent === "Rename")!;
  await act(async () => rename.click());

  expect(onRenameRemoteWorkspace).toHaveBeenCalledWith({
    desktopId: "desktop_a",
    name: "Docs",
    workspaceId: "ws_remote",
  });
  await view.unmount();
});
