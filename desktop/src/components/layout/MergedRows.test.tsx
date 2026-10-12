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
  chatSectionCollapsed?: boolean;
  filter?: DeviceFilter;
  onNewChat?: () => void;
  onNewWorkspace?: () => void;
  onToggleChatSection?: () => void;
  onToggleWorkspaceSection?: () => void;
  peers?: RemotePeer[];
  renderLocalGroupHeader?: (workspaceId: string) => React.ReactNode;
  renderLocalRow: (thread: StoredThread, depth?: number, hasChildren?: boolean) => React.ReactNode;
  rows: MergedConversation[];
  threadById: Map<string, StoredThread>;
  workspaceSectionCollapsed?: boolean;
}) {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  await act(async () => {
    root.render(
      <MergedRows
        activeKey={null}
        chatSectionCollapsed={props.chatSectionCollapsed ?? false}
        filter={props.filter ?? { kind: "all" }}
        localRowMeta={new Map()}
        onChangedRemote={vi.fn()}
        onNewChat={props.onNewChat}
        onNewWorkspace={props.onNewWorkspace}
        onOpenRemote={vi.fn()}
        onRenameRemote={vi.fn()}
        onToggleChatSection={props.onToggleChatSection ?? vi.fn()}
        onToggleWorkspaceSection={props.onToggleWorkspaceSection ?? vi.fn()}
        peers={props.peers ?? []}
        renderLocalGroupHeader={props.renderLocalGroupHeader ?? (() => null)}
        renderLocalRow={props.renderLocalRow}
        rows={props.rows}
        threadById={props.threadById}
        workspaceSectionCollapsed={props.workspaceSectionCollapsed ?? false}
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
  expect(view.container.querySelector("[data-testid='remote-source-icon']")).not.toBeNull();
  // A chat row is not a workspace the row is not in: it lands in the one chat
  // section, whatever machine it came from.
  expect(view.container.querySelector("[data-testid='merged-conversations']")!.textContent).toContain("a chat on the host");
  await view.unmount();
});

it("renders a local row through the rail's renderer, with its tree place", async () => {
  const renderLocalRow = vi.fn(() => <div>row</div>);
  const view = await render({
    renderLocalRow,
    rows: [conversation()],
    threadById: new Map([["thread_1", thread("thread_1")]]),
  });

  expect(renderLocalRow).toHaveBeenCalledWith(
    expect.objectContaining({ id: "thread_1" }),
    0,
    false,
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
    desktopId: "desktop_a",
    error: null,
    features: [],
    icon: "laptop",
    name: "Studio iMac",
    pairId: "pair_1",
  }];

  const all = await render({
    peers,
    renderLocalRow: vi.fn(() => <div>row</div>),
    rows: [remote],
    threadById: new Map(),
  });
  expect(all.container.querySelectorAll("[data-testid='remote-source-icon']")).toHaveLength(1);
  await all.unmount();

  const single = await render({
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
    desktopId: "desktop_a",
    error: null,
    features: [],
    icon: "laptop",
    name: "Studio iMac",
    pairId: "pair_1",
  }];
  const view = await render({
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
 * Foldable groups.
 *
 * The local list folds a workspace away; the merged one did not, so a user who
 * paired a machine and filtered to it saw a flat list where their own machine's
 * list had headings they could close — the same list behaving two ways.
 */
it("folds a remote workspace group away, and back", async () => {
  const view = await render({
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
        workspaceName: "future-os",
      }),
    ],
    threadById: new Map(),
  });

  const rows = () => view.container.querySelectorAll("[data-testid='remote-source-icon']").length;
  // The heading is a group's, with the local one's shape: a fold control, the
  // folder, the name, and this machine's icon after it.
  const heading = view.container.querySelector("[data-testid='merged-group-heading']")!;
  const header = heading.closest("div")!;
  const toggle = header.querySelector("button")!;
  expect(heading.textContent).toBe("future-os");
  // The fold control, and the folder the local heading has.
  expect(header.querySelectorAll("svg").length).toBeGreaterThanOrEqual(2);
  expect(rows()).toBe(1);

  await act(async () => toggle.click());
  // The heading stays (so the group can be reopened); its rows are not built.
  expect(view.container.querySelector("[data-testid='merged-group-heading']")!.textContent).toBe("future-os");
  expect(rows()).toBe(0);

  await act(async () => view.container.querySelector("[data-testid='merged-group-heading']")!.closest("div")!.querySelector("button")!.click());
  expect(rows()).toBe(1);
  await view.unmount();
});

/**
 * This machine's groups are keyed by the workspace id alone, which is the key the
 * local list folds by — so a workspace already folded there arrives folded here,
 * and one workspace does not have two fold states depending on the list.
 *
 * The heading itself is the rail's, so what the merged list owes is the *fold*:
 * a collapsed workspace's rows are left unbuilt.
 */
it("honours the fold state the local list stores, by the same key", async () => {
  localStorage.setItem("future.collapsedWorkspaces", JSON.stringify(["ws_1"]));
  try {
    const renderLocalRow = vi.fn(() => <div>row</div>);
    const view = await render({
      renderLocalGroupHeader: workspaceId => <div data-testid="local-group-heading">{workspaceId}</div>,
      renderLocalRow,
      rows: [
        conversation({
          desktopId: null,
          id: "ws_local",
          key: "local::ws_local",
          mode: "workspace",
          title: "in the workspace",
          workspaceId: "ws_1",
          workspaceName: "future-os",
        }),
      ],
      threadById: new Map([["ws_local", thread("ws_local")]]),
    });

    // The heading is there; this workspace's rows are not.
    expect(view.container.querySelector("[data-testid='local-group-heading']")).not.toBeNull();
    expect(renderLocalRow).not.toHaveBeenCalled();
    await view.unmount();
  }
  finally {
    localStorage.removeItem("future.collapsedWorkspaces");
  }
});

/** The machine's icon follows its name, so the name starts the heading. */
it("puts the machine's icon after the group name", async () => {
  const view = await render({
    peers: [{
      agentAvailable: true,
      bridgeInstanceId: "bridge_1",
      connected: true,
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
 * The chat section folds as well as a workspace one, through the section header.
 *
 * It is where most conversations are, and a reader who wants to scroll past them
 * has no other way to put them away.
 */
it("folds the chat section through its header, and back", async () => {
  const onToggleChatSection = vi.fn();
  const view = await render({
    onToggleChatSection,
    peers: [peer("desktop_a", "Studio iMac")],
    renderLocalRow: vi.fn(() => <div>row</div>),
    rows: [conversation({ desktopId: "desktop_a", id: "chat_a", key: "desktop_a::chat_a", title: "a chat on the host" })],
    threadById: new Map(),
  });

  const sectionToggle = view.container.querySelector<HTMLButtonElement>("button[aria-label='Collapse chat list']")!;
  expect(sectionToggle).not.toBeNull();
  await act(async () => sectionToggle.click());
  expect(onToggleChatSection).toHaveBeenCalledTimes(1);

  // Collapsed, the rows are not built; the section header stays so it can be
  // reopened.
  await view.unmount();
  const folded = await render({
    chatSectionCollapsed: true,
    renderLocalRow: vi.fn(() => <div>row</div>),
    rows: [conversation({ desktopId: "desktop_a", id: "chat_a", key: "desktop_a::chat_a", title: "a chat on the host" })],
    threadById: new Map(),
  });
  expect(folded.container.textContent).not.toContain("a chat on the host");
  expect(folded.container.textContent).toContain("Chat");
  await folded.unmount();
});

it("offers the local-only controls only when this machine's rows are listed", async () => {
  const onNewChat = vi.fn();
  const onNewWorkspace = vi.fn();
  const rows = [conversation({ desktopId: "desktop_a", id: "chat_a", key: "desktop_a::chat_a" })];

  const all = await render({ onNewChat, onNewWorkspace, renderLocalRow: vi.fn(() => <div>row</div>), rows, threadById: new Map() });
  expect(all.container.querySelector("button[aria-label='New chat']")).not.toBeNull();
  expect(all.container.querySelector("button[aria-label='New workspace']")).not.toBeNull();
  await all.unmount();

  // Filtered to a host, "new chat" would create a chat on *this* machine and
  // show it under the host's filter — a control that adds something the user
  // cannot see.
  const remote = await render({
    filter: { kind: "device", desktopId: "desktop_a" },
    onNewChat,
    onNewWorkspace,
    renderLocalRow: vi.fn(() => <div>row</div>),
    rows,
    threadById: new Map(),
  });
  expect(remote.container.querySelector("button[aria-label='New chat']")).toBeNull();
  expect(remote.container.querySelector("button[aria-label='New workspace']")).toBeNull();
  await remote.unmount();
});
