// @vitest-environment jsdom
import type { ComponentProps, ReactElement } from "react";
import type { StoredThread, StoredWorkspace } from "../../integrations/storage/threadStore";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { resetAllCommandBindings } from "../../features/keyboard/shortcuts";
import { installShortcuts } from "../../test/installShortcuts";
import { flushAsync } from "../../test/renderHook";
import { ActivityRail } from "./ActivityRail";

vi.mock("../../integrations/skills/skillsClient", () => ({ listInstalledSkills: async () => [] }));
vi.mock("../../integrations/agent/agentStateCache", () => ({ useCachedAgentState: () => undefined }));
vi.mock("../../lib/useIsFullscreen", () => ({ useIsFullscreen: () => false }));
vi.mock("./hooks/usePendingApprovalCounts", () => ({ usePendingApprovalCounts: () => new Map() }));
vi.mock("../../lib/useFloatingScrollbar", () => ({
  useFloatingScrollbar: () => ({ scrollRef: { current: null }, scrollbar: { height: 0, top: 0, visible: false }, handleScroll: () => {}, handleThumbPointerDown: () => {} }),
}));

function thread(id: string, overrides: Partial<StoredThread> = {}): StoredThread {
  return {
    id,
    agentSessionId: id,
    title: id,
    mode: "chat",
    workspaceId: id,
    status: "active",
    pinned: false,
    readonly: false,
    createdAt: 0,
    updatedAt: 0,
    ...overrides,
  };
}

const beta: StoredWorkspace = {
  cleanupStatus: "active",
  createdAt: 0,
  id: "ws-b",
  kind: "user",
  name: "Beta",
  path: "/tmp/beta",
  updatedAt: 0,
};

const alpha: StoredWorkspace = {
  cleanupStatus: "active",
  createdAt: 0,
  id: "ws-a",
  kind: "user",
  name: "Alpha",
  path: "/tmp/alpha",
  updatedAt: 0,
};

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

function props(threads: StoredThread[], overrides: Partial<ComponentProps<typeof ActivityRail>> = {}): ComponentProps<typeof ActivityRail> {
  return {
    active: "chat" as const,
    activeThreadId: "root",
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
    workspaces: [alpha],
    ...overrides,
  };
}

function buttonByText(container: HTMLElement, text: string) {
  return [...container.querySelectorAll<HTMLButtonElement>("button")].find(button => button.textContent === text);
}

/**
 * The complete set of labelled buttons in the expanded rail's nav, in render
 *  order. Discovered from a deliberate mismatch while writing the guard test
 *  below; the trailing two come from that test's workspace/account fixtures.
 */
// The two view-switch entries sit between the standing entries and the list
// they switch: "Workspaces" and "Recent" are one property of the list, and the
// switch is drawn in the nav's own column. They are in this set on purpose — the
// assertion's job is to make every addition here deliberate.
const EXPECTED_NAV_ENTRIES = ["New Chat", "Models", "Skills", "Scheduled tasks", "Remote", "Workspaces", "Recent", "Alpha", "Aalice"];

beforeEach(() => {
  localStorage.clear();
});
afterEach(() => {
  localStorage.clear();
  vi.useRealTimers();
});

describe("activity rail navigation", () => {
  it("routes every expanded nav entry and the workspace/chat '+' buttons", () => {
    const p = props([thread("root"), thread("ws-thread", { mode: "workspace", workspaceId: "ws-a" })]);
    const view = mount(<ActivityRail {...p} />);

    act(() => buttonByText(view.container, "New Chat")!.click());
    expect(p.onNewChat).toHaveBeenCalledWith();
    act(() => buttonByText(view.container, "Models")!.click());
    expect(p.onOpenModels).toHaveBeenCalledTimes(1);
    act(() => buttonByText(view.container, "Skills")!.click());
    expect(p.onChange).toHaveBeenCalledWith("skill");
    act(() => buttonByText(view.container, "Remote")!.click());
    expect(p.onChange).toHaveBeenLastCalledWith("remote");
    act(() => buttonByText(view.container, "Scheduled tasks")!.click());
    expect(p.onChange).toHaveBeenLastCalledWith("tasks");

    act(() => view.container.querySelector<HTMLButtonElement>("button[aria-label=\"New workspace\"]")!.click());
    expect(p.onNewWorkspace).toHaveBeenCalledTimes(1);
    act(() => view.container.querySelector<HTMLButtonElement>("button[aria-label=\"New chat\"]")!.click());
    expect(p.onNewChat).toHaveBeenCalledTimes(2);
    act(() => view.container.querySelector<HTMLButtonElement>("button[aria-label=\"New chat in Alpha\"]")!.click());
    expect(p.onNewChat).toHaveBeenLastCalledWith("ws-a");
    view.unmount();
  });

  it("renders exactly the expected set of nav entries", () => {
    // A *set* assertion, not a spot check. The sibling test above clicks each
    // entry by name, which proves those entries exist but says nothing about
    // extras — and `ActivityRail` has a `featureItems` array that is currently
    // empty but is documented as "add them back to restore" (`ActivityRail.tsx`
    // L88-92). Injecting one entry there makes a new nav button appear, and
    // before this test existed **55 of 55 rail tests still passed**.
    //
    // The list below is the complete set of labelled buttons in the nav, in
    // render order: the four section entries (each wired to a different handler,
    // which is why they cannot be isolated by "who calls onChange"), then the
    // workspace group header and the account button. The order is part of the
    // expectation: Scheduled tasks sit with the standing entries, and Remote follows them.
    // The two directions of remote used to be two entries — labels a word apart
    // that neither distinguished nor even described accurately; they are one
    // entry now, and the page inside separates them.
    // of the same idea, and are now one page that separates them in words.
    // The last two come from this test's own fixtures (workspace "Alpha", email
    // "alice@example.com"), so the expectation is deterministic; a deliberate UI
    // addition updates this line.
    const view = mount(<ActivityRail {...props([thread("root")])} />);
    const nav = view.container.querySelector("nav")!;
    const labels = [...nav.querySelectorAll<HTMLButtonElement>("button")]
      .map(button => (button.textContent ?? "").trim())
      .filter(Boolean);
    expect(labels).toEqual(EXPECTED_NAV_ENTRIES);
    view.unmount();
  });

  it("keeps Remote reachable while signed out", () => {
    // The entry used to be hidden unless signed in, because pairing the *host*
    // side issues its code through the FutureOS service. But the client half
    // needs no sign-in at all, so hiding the entry hid a feature that worked —
    // and the section that does need an account says so itself, inside.
    const view = mount(<ActivityRail {...props([thread("root")], { futureSessionStatus: "signed_out" })} />);
    expect(buttonByText(view.container, "Remote")).toBeTruthy();
    view.unmount();
  });

  it("routes the collapsed rail's icon buttons", () => {
    const p = props([thread("root")], { expanded: false });
    const view = mount(<ActivityRail {...p} />);
    act(() => view.container.querySelector<HTMLButtonElement>("button[aria-label=\"New chat\"]")!.click());
    expect(p.onNewChat).toHaveBeenCalledWith();
    act(() => view.container.querySelector<HTMLButtonElement>("button[aria-label=\"Models\"]")!.click());
    expect(p.onOpenModels).toHaveBeenCalledTimes(1);
    act(() => view.container.querySelector<HTMLButtonElement>("button[aria-label=\"Remote\"]")!.click());
    expect(p.onChange).toHaveBeenCalledWith("remote");
    act(() => view.container.querySelector<HTMLButtonElement>("button[aria-label=\"Scheduled tasks\"]")!.click());
    expect(p.onChange).toHaveBeenLastCalledWith("tasks");
    act(() => view.container.querySelector<HTMLButtonElement>("button[aria-label=\"Workspace\"]")!.click());
    expect(p.onChange).toHaveBeenCalledWith("workspace");
    act(() => view.container.querySelector<HTMLButtonElement>("button[aria-label=\"Chat\"]")!.click());
    expect(p.onChange).toHaveBeenCalledWith("chat");
    view.unmount();
  });

  it("collapses and re-expands each list section independently", () => {
    const p = props([thread("root"), thread("ws-thread", { mode: "workspace", workspaceId: "ws-a" })]);
    const view = mount(<ActivityRail {...p} />);
    const has = (title: string) => view.container.querySelector(`button[aria-label="${title}"]`) !== null;

    expect(has("root")).toBe(true);
    expect(has("ws-thread")).toBe(true);
    act(() => view.container.querySelector<HTMLButtonElement>("button[aria-label=\"Collapse chat list\"]")!.click());
    expect(has("root")).toBe(false);
    // Workspace section is independent — its rows stay.
    expect(has("ws-thread")).toBe(true);
    act(() => view.container.querySelector<HTMLButtonElement>("button[aria-label=\"Expand chat list\"]")!.click());
    expect(has("root")).toBe(true);

    act(() => view.container.querySelector<HTMLButtonElement>("button[aria-label=\"Collapse workspace list\"]")!.click());
    expect(has("ws-thread")).toBe(false);
    expect(has("root")).toBe(true);
    view.unmount();
  });
});

describe("activity rail workspace group actions", () => {
  it("lists a non-user workspace that still owns conversations", () => {
    const shared = { ...alpha, id: "ws-shared", kind: "shared", name: "Shared" } as unknown as StoredWorkspace;
    const p = props([thread("shared-thread", { mode: "workspace", workspaceId: "ws-shared" })], { workspaces: [shared] });
    const view = mount(<ActivityRail {...p} />);
    expect([...view.container.querySelectorAll("button")].some(button => button.textContent === "Shared")).toBe(true);
    view.unmount();
  });

  it("selects the workspace, opens its menu on right-click and starts a chat inside it", () => {
    const p = props([thread("ws-thread", { mode: "workspace", workspaceId: "ws-a" })]);
    const view = mount(<ActivityRail {...p} />);
    const nameButton = [...view.container.querySelectorAll<HTMLButtonElement>("button")]
      .find(button => button.textContent === "Alpha")!;

    act(() => nameButton.click());
    expect(p.onSelectWorkspace).toHaveBeenCalledWith(alpha, [expect.objectContaining({ id: "ws-thread" })]);

    const header = nameButton.parentElement!;
    act(() => {
      header.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true }));
    });
    expect(view.container.querySelector("[role=\"menu\"]")).not.toBeNull();
    view.unmount();
  });

  it("enters selection mode for a workspace from its header menu", async () => {
    const p = props([
      thread("ws-a", { mode: "workspace", workspaceId: "ws-a" }),
      thread("ws-b", { mode: "workspace", workspaceId: "ws-a" }),
    ]);
    const view = mount(<ActivityRail {...p} />);
    const nameButton = [...view.container.querySelectorAll<HTMLButtonElement>("button")]
      .find(button => button.textContent === "Alpha")!;
    act(() => {
      nameButton.parentElement!.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true }));
    });
    const selectChats = [...view.container.querySelectorAll<HTMLButtonElement>("[role=\"menuitem\"]")]
      .find(item => item.textContent === "Select chats")!;
    act(() => selectChats.click());
    await flushAsync();

    // Selection mode replaces the rows' actions with checkboxes.
    expect(view.container.querySelector("input[type=\"checkbox\"]")).not.toBeNull();
    view.unmount();
  });
});

describe("activity rail thread menu", () => {
  it("opens a thread's actions menu from the row and routes its items", () => {
    const p = props([thread("root")]);
    const view = mount(<ActivityRail {...p} />);
    const trigger = view.container.querySelector<HTMLButtonElement>("button[aria-label=\"Thread actions for root\"]")!;
    expect(trigger.getAttribute("aria-expanded")).toBe("false");

    act(() => trigger.click());
    expect(view.container.querySelector<HTMLButtonElement>("button[aria-label=\"Thread actions for root\"]")!.getAttribute("aria-expanded")).toBe("true");
    const rename = [...view.container.querySelectorAll<HTMLButtonElement>("[role=\"menuitem\"]")]
      .find(item => item.textContent === "Rename")!;
    act(() => rename.click());
    expect(p.onRenameThread).toHaveBeenCalledWith(expect.objectContaining({ id: "root" }));
    expect(view.container.querySelector("[role=\"menu\"]")).toBeNull();
    view.unmount();
  });
});

describe("activity rail skill attention", () => {
  it("routes the intro bubble's go action to the Skills page", async () => {
    const p = props([thread("root")], { skillIntroDismissed: false });
    const view = mount(<ActivityRail {...p} />);
    // The bubble waits for the installed-skills count to land.
    expect(buttonByText(view.container, "Take a look")).toBeUndefined();
    await flushAsync();
    act(() => buttonByText(view.container, "Take a look")!.click());
    expect(p.onDismissSkillIntro).toHaveBeenCalledTimes(1);
    expect(p.onChange).toHaveBeenCalledWith("skill");
    view.unmount();
  });

  it("pulses the Skills entry after the guide is dismissed and stops after 4s", async () => {
    vi.useFakeTimers();
    const view = mount(<ActivityRail {...props([thread("root")])} />);
    await act(async () => {
      await Promise.resolve();
    });
    const skillsClass = () => buttonByText(view.container, "Skills")!.className;
    const before = skillsClass();

    await act(async () => {
      window.dispatchEvent(new CustomEvent("futureos:skill-guide-dismissed", { detail: undefined }));
      await Promise.resolve();
    });
    expect(skillsClass()).not.toBe(before);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(4000);
    });
    expect(skillsClass()).toBe(before);
    view.unmount();
  });
});

/**
 * The list's own keyboard commands: walking the rows the rail draws, and jumping
 * to the filter box. Both are registered here because the order they act on is
 * this list's, and a copy of that order in the shell would be a copy that drifts.
 */
describe("activity rail keyboard commands", () => {
  let uninstallShortcuts: () => void;

  beforeEach(() => {
    resetAllCommandBindings();
    uninstallShortcuts = installShortcuts();
  });
  afterEach(() => {
    uninstallShortcuts();
  });

  function press(init: KeyboardEventInit) {
    act(() => {
      window.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, cancelable: true, ...init }));
    });
  }

  it("walks to the next and previous conversation in the drawn order", () => {
    const p = props([thread("root"), thread("second")]);
    const view = mount(<ActivityRail {...p} />);

    press({ code: "BracketRight", ctrlKey: true, key: "]", shiftKey: true });
    expect(p.onSelectThread).toHaveBeenLastCalledWith(expect.objectContaining({ id: "second" }));

    // The shell owns "current": the rail only reports the row the user landed on,
    // so the next step is driven by the selection it hands back down.
    view.rerender(<ActivityRail {...p} activeThreadId="second" />);
    press({ code: "BracketLeft", ctrlKey: true, key: "[", shiftKey: true });
    expect(p.onSelectThread).toHaveBeenLastCalledWith(expect.objectContaining({ id: "root" }));
    view.unmount();
  });

  it("opens the first conversation when nothing is selected", () => {
    const p = props([thread("root"), thread("second")], { activeThreadId: null });
    const view = mount(<ActivityRail {...p} />);
    press({ code: "BracketRight", ctrlKey: true, key: "]" });
    expect(p.onSelectThread).toHaveBeenCalledTimes(1);
    view.unmount();
  });

  it("stops at the end of the list instead of wrapping", () => {
    const p = props([thread("root")]);
    const view = mount(<ActivityRail {...p} />);
    press({ code: "BracketRight", ctrlKey: true, key: "]", shiftKey: true });
    expect(p.onSelectThread).not.toHaveBeenCalled();
    view.unmount();
  });

  it("leaves the keys unclaimed on a section with no list", () => {
    const p = props([thread("root"), thread("second")], { active: "skill" });
    const view = mount(<ActivityRail {...p} />);
    const event = new KeyboardEvent("keydown", { bubbles: true, cancelable: true, code: "BracketRight", ctrlKey: true, key: "]", shiftKey: true });
    act(() => window.dispatchEvent(event));
    expect(p.onSelectThread).not.toHaveBeenCalled();
    // Not ours, so it is not consumed either.
    expect(event.defaultPrevented).toBe(false);
    view.unmount();
  });

  /**
   * The command is a focus now, not an open.
   *
   * The field is always drawn (see `ActivityRail.toolbar.test.tsx` for the
   * structure that guarantees it), so what the key has to do is put the caret in
   * it — which is the only thing that was useful about it before.
   */
  it("puts the caret in the filter box", async () => {
    const p = props([thread("root"), thread("second")]);
    const view = mount(<ActivityRail {...p} />);
    const field = () => view.container.querySelector("input[aria-label=\"Search conversations\"]");
    // Present with nothing done, and not where the caret is.
    expect(field()).not.toBeNull();
    expect(document.activeElement).not.toBe(field());

    press({ code: "KeyF", ctrlKey: true, key: "f", shiftKey: true });
    await act(async () => {});
    expect(document.activeElement).toBe(field());
    view.unmount();
  });
});

/**
 * The two navigation levels, and the reason they are two.
 *
 * The rail lists two kinds of thing — workspaces (which hold conversations) and
 * conversations — in two sections. The coarse move (`group.*`) steps between
 * those*, crossing the section boundary because a chat and a workspace are
 * siblings in the user's model; the fine move (`thread.*`) walks the drawn
 * conversation rows, which is "within a workspace" or "within a conversation's
 * forks". Both were asked for together, so they are measured together.
 */
describe("activity rail two-level navigation", () => {
  let uninstallShortcuts: () => void;

  beforeEach(() => {
    resetAllCommandBindings();
    uninstallShortcuts = installShortcuts();
  });
  afterEach(() => {
    uninstallShortcuts();
  });

  function press(init: KeyboardEventInit) {
    act(() => {
      window.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, cancelable: true, ...init }));
    });
  }

  const coarseNext = () => press({ code: "BracketRight", ctrlKey: true, key: "]", shiftKey: true });
  const coarsePrevious = () => press({ code: "BracketLeft", ctrlKey: true, key: "[", shiftKey: true });
  const fineNext = () => press({ code: "BracketRight", ctrlKey: true, key: "]" });
  const finePrevious = () => press({ code: "BracketLeft", ctrlKey: true, key: "[" });

  /** A chat conversation, two in one workspace, and a second workspace. */
  function fixture() {
    return [
      thread("chat-a"),
      thread("alpha-1", { mode: "workspace", workspaceId: "ws-a" }),
      thread("alpha-2", { mode: "workspace", workspaceId: "ws-a" }),
      thread("beta-1", { mode: "workspace", workspaceId: "ws-b" }),
    ];
  }

  it("steps between workspaces and conversations, skipping the rows inside", () => {
    const p = props(fixture(), { active: "workspace", activeThreadId: "alpha-1", workspaces: [alpha, beta] });
    const view = mount(<ActivityRail {...p} />);

    // The drawn order is pinned, then the workspaces, then the chats — so from a
    // workspace the coarse move reaches the next workspace, not its second
    // conversation.
    coarseNext();
    expect(p.onSelectThread).toHaveBeenLastCalledWith(expect.objectContaining({ id: "beta-1" }));

    // … and then the chat section, which the coarse move crosses on purpose:
    // opening a conversation is what switches the section.
    view.rerender(<ActivityRail {...p} activeThreadId="beta-1" />);
    coarseNext();
    expect(p.onSelectThread).toHaveBeenLastCalledWith(expect.objectContaining({ id: "chat-a" }));

    // Backwards, the same stops in reverse.
    view.rerender(<ActivityRail {...p} activeThreadId="chat-a" />);
    coarsePrevious();
    expect(p.onSelectThread).toHaveBeenLastCalledWith(expect.objectContaining({ id: "beta-1" }));

    // A conversation nested inside a workspace is *on* that workspace's stop, so
    // "previous" from it is the stop before the workspace, not the workspace
    // itself.
    view.rerender(<ActivityRail {...p} activeThreadId="beta-1" />);
    coarsePrevious();
    expect(p.onSelectThread).toHaveBeenLastCalledWith(expect.objectContaining({ id: "alpha-1" }));
    view.rerender(<ActivityRail {...p} activeThreadId="alpha-2" />);
    coarsePrevious();
    // alpha is the first stop: nowhere further back.
    expect(p.onSelectThread).toHaveBeenCalledTimes(4);
    view.unmount();
  });

  it("walks the conversations inside the current list", () => {
    const p = props(fixture(), { active: "workspace", activeThreadId: "alpha-1", workspaces: [alpha, beta] });
    const view = mount(<ActivityRail {...p} />);

    fineNext();
    expect(p.onSelectThread).toHaveBeenLastCalledWith(expect.objectContaining({ id: "alpha-2" }));

    view.rerender(<ActivityRail {...p} activeThreadId="alpha-2" />);
    fineNext();
    expect(p.onSelectThread).toHaveBeenLastCalledWith(expect.objectContaining({ id: "beta-1" }));

    view.rerender(<ActivityRail {...p} activeThreadId="beta-1" />);
    finePrevious();
    expect(p.onSelectThread).toHaveBeenLastCalledWith(expect.objectContaining({ id: "alpha-2" }));
    view.unmount();
  });

  it("stops at both ends of the coarse walk", () => {
    const p = props(fixture(), { active: "workspace", activeThreadId: "alpha-1", workspaces: [alpha, beta] });
    const view = mount(<ActivityRail {...p} />);
    // alpha is the first stop (no pinned conversations here);
    coarsePrevious();
    expect(p.onSelectThread).not.toHaveBeenCalled();

    view.rerender(<ActivityRail {...p} activeThreadId="chat-a" />);
    coarseNext();
    expect(p.onSelectThread).not.toHaveBeenCalled();
    view.unmount();
  });

  it("leaves the coarse keys unclaimed when there is only one stop", () => {
    const p = props([thread("only")], { workspaces: [] });
    const view = mount(<ActivityRail {...p} />);
    // One conversation and nothing else: nowhere to step, so the key is not
    // consumed and stays free for the platform.
    const event = new KeyboardEvent("keydown", { bubbles: true, cancelable: true, code: "BracketRight", ctrlKey: true, key: "]", shiftKey: true });
    act(() => window.dispatchEvent(event));
    expect(p.onSelectThread).not.toHaveBeenCalled();
    expect(event.defaultPrevented).toBe(false);
    view.unmount();
  });

  it("lands on a workspace's first conversation, and opens an empty one", () => {
    const empty: StoredWorkspace = { ...beta, id: "ws-empty", name: "Empty", path: "/tmp/empty" };
    const p = props(fixture(), { active: "workspace", activeThreadId: "alpha-1", workspaces: [alpha, empty] });
    const view = mount(<ActivityRail {...p} />);

    // A workspace with no conversations is still a stop: it is selected as a
    // workspace rather than skipped, which is how the rail lists it.
    coarseNext();
    expect(p.onSelectWorkspace).toHaveBeenCalledWith(expect.objectContaining({ id: "ws-empty" }), []);
    expect(p.onSelectThread).not.toHaveBeenCalled();
    view.unmount();
  });
});

/**
 * The same two levels with a machine paired, where the list is the merged one.
 *
 * This is not a variation for its own sake: the merged list is the one a user
 * with a second computer actually navigates, and its rows are not the local
 * list's — they are keyed `local::<id>` / `<desktopId>::<sessionId>` and grouped
 * by `groupMergedRows`. The coarse walk first shipped against the local shapes
 * only, and in a paired window it stalled on the conversation it started from.
 */
describe("two-level navigation in the merged list", () => {
  let uninstallShortcuts: () => void;

  beforeEach(() => {
    resetAllCommandBindings();
    uninstallShortcuts = installShortcuts();
  });
  afterEach(() => {
    uninstallShortcuts();
  });

  function press(init: KeyboardEventInit) {
    act(() => {
      window.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, cancelable: true, ...init }));
    });
  }

  const coarseNext = () => press({ code: "BracketRight", ctrlKey: true, key: "]", shiftKey: true });
  const fineNext = () => press({ code: "BracketRight", ctrlKey: true, key: "]" });

  /**
   * Two conversations in `alpha`, one in `beta`, one chat — timestamps set, so the
   * drawn order is `alpha-1`, `alpha-2`, `beta-1`, `chat-a` and the assertions can
   * name rows instead of "anything but".
   */
  async function mountMerged() {
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    const base = props(
      [
        thread("alpha-1", { lastMessageAt: 4_000, mode: "workspace", updatedAt: 4_000, workspaceId: "ws-a" }),
        thread("alpha-2", { lastMessageAt: 3_000, mode: "workspace", updatedAt: 3_000, workspaceId: "ws-a" }),
        thread("beta-1", { lastMessageAt: 2_000, mode: "workspace", updatedAt: 2_000, workspaceId: "ws-b" }),
        thread("chat-a", { lastMessageAt: 1_000, updatedAt: 1_000 }),
      ],
      {
        active: "workspace",
        activeThreadId: "alpha-1",
        onOpenRemoteConversation: vi.fn(),
        workspaces: [alpha, beta],
      },
    );
    const all = {
      ...base,
      // A paired host is what puts the rail into merged mode.
      remotePeers: [{
        agentAvailable: true,
        bridgeInstanceId: "bridge_1",
        connected: true,
        desktopId: "desktop_a",
        error: null,
        features: [],
        icon: "laptop",
        name: "Studio iMac",
        pairId: "pair_1",
        stopped: false,
      }],
      remoteCatalogs: [{
        desktopId: "desktop_a",
        sessions: [
          { lastMessageAt: 500, sessionId: "remote-1", threadId: "remote-1_thread", title: "Remote one" },
        ],
      }],
    } as unknown as Required<Parameters<typeof props>[1]>;
    act(() => root.render(<ActivityRail {...(all as unknown as ComponentProps<typeof ActivityRail>)} />));
    await flushAsync();
    return {
      all,
      /** Re-render as the shell does: the opened row is the new current one. */
      rerender: (next: Parameters<typeof props>[1]) => act(() => root.render(
        <ActivityRail {...(all as unknown as ComponentProps<typeof ActivityRail>)} {...next} />,
      )),
      selected: () => {
        const threadCall = (all!.onSelectThread as unknown as ReturnType<typeof vi.fn>).mock.calls;
        const remoteCall = (all!.onOpenRemoteConversation as unknown as ReturnType<typeof vi.fn>).mock.calls;
        return {
          calls: threadCall.length + remoteCall.length,
          id: (threadCall.slice(-1)[0]?.[0] as { id?: string } | undefined)?.id
            ?? (remoteCall.slice(-1)[0]?.[0] as { id?: string } | undefined)?.id,
        };
      },
      unmount: () => {
        act(() => root.unmount());
        container.remove();
      },
    };
  }

  it("steps past a workspace's other conversations, and walks rows one at a time", async () => {
    const view = await mountMerged();

    // From the first conversation of `alpha`: the coarse move leaves the
    // workspace, the fine move takes the next row of it.
    coarseNext();
    await flushAsync();
    expect(view.selected().id).toBe("beta-1");

    view.rerender({ activeThreadId: "alpha-1" });
    fineNext();
    await flushAsync();
    expect(view.selected().id).toBe("alpha-2");
    view.unmount();
  });

  it("walks the section's rows in the order the merged list draws them", async () => {
    // The list regroups its rows by workspace, so the drawn order is the grouped
    // one. The first version of the fine move walked the tree order instead, and
    // the two levels then disagreed about where a workspace began: `⇧]` skipped a
    // conversation the plain `]` walked into. Both levels now read one block list.
    const view = await mountMerged();
    const visited: string[] = [];
    for (let step = 0; step < 3; step += 1) {
      fineNext();
      await flushAsync();
      const landed = view.selected().id;
      // No call means the walk had nowhere to go; keep the row so the assertion
      // below says "it stopped" rather than quietly starting over.
      visited.push(landed ?? visited[visited.length - 1] ?? "alpha-1");
      view.rerender({ activeThreadId: landed ?? "beta-1" });
    }
    // The workspace section holds the two workspaces' rows and nothing else: it
    // walks them in drawn order and stops at the end instead of wrapping.
    expect(visited).toEqual(["alpha-2", "beta-1", "beta-1"]);
    view.unmount();
  });
});
