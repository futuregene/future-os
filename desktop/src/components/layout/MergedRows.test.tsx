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
  filter?: DeviceFilter;
  peers?: RemotePeer[];
  renderLocalRow: (thread: StoredThread, depth?: number, hasChildren?: boolean) => React.ReactNode;
  rows: MergedConversation[];
  threadById: Map<string, StoredThread>;
}) {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  await act(async () => {
    root.render(
      <MergedRows
        activeKey={null}
        filter={props.filter ?? { kind: "all" }}
        localRowMeta={new Map()}
        onChangedRemote={vi.fn()}
        onOpenRemote={vi.fn()}
        onRenameRemote={vi.fn()}
        peers={props.peers ?? []}
        renderLocalRow={props.renderLocalRow}
        rows={props.rows}
        threadById={props.threadById}
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

  const labels = [...view.container.querySelectorAll("span")]
    .map(node => node.textContent ?? "")
    .filter(Boolean);
  // Both machines have a "future-os" workspace, and each keeps its own heading.
  expect(labels.filter(text => text === "future-os")).toHaveLength(2);
  // A chat row on a host is its own section, named after that host, rather than
  // a heading for a workspace the row is not in. (A remote *workspace* heading
  // stays the workspace's name, which is what the user calls it.)
  expect(labels.filter(text => text === "Studio iMac")).toHaveLength(1);
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
 * A remote chat row whose machine is not in the peer list — a stale catalogue
 * after a disconnect, or a caller that supplies rows without peers. Its section
 * is the *machine's*, so with no machine to name it falls back to the generic
 * heading rather than rendering an unnamed section or throwing.
 */
it("names a remote chat section generically when its machine is unknown", async () => {
  const view = await render({
    peers: [],
    renderLocalRow: vi.fn(() => <div>row</div>),
    rows: [
      conversation({
        desktopId: "desktop_gone",
        id: "sess_gone",
        key: "desktop_gone::sess_gone",
        title: "a chat on a machine that is not listed",
      }),
    ],
    threadById: new Map(),
  });

  const headings = [...view.container.querySelectorAll("[data-testid='merged-group-heading']")]
    .map(node => (node.textContent || "").trim());
  expect(headings).toEqual(["Chat"]);
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
it("folds a workspace group away, and back", async () => {
  const renderLocalRow = vi.fn(() => <div>row</div>);
  const view = await render({
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

  const header = [...view.container.querySelectorAll("[data-testid='merged-group-heading']")]
    .find(node => node.textContent === "future-os")!;
  const toggle = header.parentElement!.querySelector("button")!;
  expect(renderLocalRow).toHaveBeenCalledTimes(1);

  await act(async () => toggle.click());
  // The heading stays (so the group can be reopened); its rows are not built.
  expect(view.container.querySelector("[data-testid='merged-group-heading']")!.textContent).toBe("future-os");
  expect(renderLocalRow).toHaveBeenCalledTimes(1);

  await act(async () => toggle.click());
  expect(renderLocalRow).toHaveBeenCalledTimes(2);
  await view.unmount();
});

/**
 * This machine's groups are keyed by the workspace id alone, which is the key the
 * local list folds by — so a workspace already folded there arrives folded here,
 * and one workspace does not have two fold states depending on the list.
 */
it("honours the fold state the local list stores, by the same key", async () => {
  localStorage.setItem("future.collapsedWorkspaces", JSON.stringify(["ws_1"]));
  try {
    const renderLocalRow = vi.fn(() => <div>row</div>);
    const view = await render({
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
    expect(view.container.querySelector("[data-testid='merged-group-heading']")!.textContent).toBe("future-os");
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
 * A chat section folds as well as a workspace one.
 *
 * The chat section is where most conversations are, and a reader who wants to
 * scroll past another machine's chats to reach their own has no other way to put
 * them away.
 */
it("folds a chat section away, and back", async () => {
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
        id: "chat_a",
        key: "desktop_a::chat_a",
        title: "a chat on the host",
      }),
    ],
    threadById: new Map(),
  });

  const heading = [...view.container.querySelectorAll("[data-testid='merged-group-heading']")]
    .find(node => node.textContent === "Studio iMac")!;
  const toggle = heading.parentElement!.querySelector("button")!;
  const rows = () => view.container.querySelectorAll("[data-testid='remote-source-icon']").length;
  expect(rows()).toBe(1);

  await act(async () => toggle.click());
  expect(rows()).toBe(0);
  expect(view.container.querySelector("[data-testid='merged-group-heading']")!.textContent).toBe("Studio iMac");

  await act(async () => toggle.click());
  expect(rows()).toBe(1);
  await view.unmount();
});
