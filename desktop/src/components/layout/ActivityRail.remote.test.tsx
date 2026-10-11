// @vitest-environment jsdom
import type { ComponentProps } from "react";
import type { StoredThread } from "../../integrations/storage/threadStore";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { beforeEach, describe, expect, it, vi } from "vitest";
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
  desktopId: "desktop_a",
  error: null,
  features: [],
  icon: "laptop",
  name: "Studio iMac",
  pairId: "pair_1",
};

function railProps(overrides: Partial<ComponentProps<typeof ActivityRail>> = {}) {
  return {
    active: "chat" as const,
    activeThreadId: null,
    expanded: true,
    onBatchDeleteThreads: vi.fn(),
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
    text: () => container.textContent ?? "",
    unmount: () => {
      act(() => root.unmount());
      container.remove();
    },
  };
}

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
      remoteCatalogs: [catalogRow("sess_remote", { lastMessageAt: 9_000 })],
      remotePeers: [peer],
      threads: [thread("local_newer", { lastMessageAt: 10_000, title: "Local newer", updatedAt: 10_000 })],
    });

    const titles = [...view.container.querySelectorAll("[title]")]
      .map(node => node.getAttribute("title")!.split(" · ")[0]);
    expect(titles).toContain("Local newer");
    expect(titles).toContain("Remote conversation");
    // Newer first, regardless of which machine it lives on.
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
});
