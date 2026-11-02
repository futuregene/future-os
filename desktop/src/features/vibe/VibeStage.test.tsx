import type { StoredThread, StoredWorkspace } from "../../integrations/storage/threadStore";
// @vitest-environment jsdom
import type { RemoteCatalog } from "../remote-peer/mergeConversations";
import type { RemotePeer } from "../remote-peer/remotePeerClient";
import type { VibeThreadViewProps } from "./VibeStage";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { onFutureEvent } from "../../lib/futureEvents";
import { isMacOS } from "../../lib/platform";
import { installShortcuts } from "../../test/installShortcuts";
import { loadComposerDraft, saveComposerDraft } from "../agent/composerDraft";
import { clearCommandHandlers, runCommand } from "../keyboard/commands";
import { resetShortcutScopes } from "../keyboard/scopes";
import { resetAllCommandBindings } from "../keyboard/shortcuts";
import { gridOf } from "./layout";
import { clearVibePreferences, readVibePreferences, writeVibeDevice } from "./vibePreferences";
import { VibeStage } from "./VibeStage";

/**
 * The panel's own logic: which layer owns the screen, which conversation each
 * layer draws, and what order the cells are in.
 *
 * `AgentThread` is stubbed **by chrome** — the only thing this file is entitled to
 * assert about it — and the stub renders one editable element for the input layer
 * and none for the reading layer, which is the contract the real component
 * implements (`AgentThreadChrome.test.tsx` holds that half). Together the two
 * files prove the panel's invariant — at most one editable element — without
 * mounting a conversation's whole machinery three times.
 */
const h = vi.hoisted(() => ({
  approvals: {
    activeApproval: null as unknown,
    decideApproval: vi.fn<(approval: unknown, status: string) => Promise<void>>(async () => {}),
  },
  counts: new Map<string, number>(),
  cached: { current: null as { model?: string; thinkingLevel?: string } | null },
  /** What the panel's queue answered when the stub composer handed a sentence over. */
  probe: { taken: undefined as boolean | undefined },
  /** The panel's two writers for a cell's model/level (mocked, see below). */
  setters: {
    model: vi.fn<(threadId: string, modelId: string, options: unknown) => Promise<void>>(async () => {}),
    thinking: vi.fn<(threadId: string, level: string) => Promise<void>>(async () => {}),
  },
  /** The one window object the panel sees (see the mock above). */
  win: {
    isFullscreen: vi.fn(async () => false),
    onResized: vi.fn(async () => () => {}),
    setFullscreen: vi.fn(async () => {}),
  },
}));

vi.mock("./vibeCellSettings", async (importOriginal) => {
  const actual = await importOriginal<typeof import("./vibeCellSettings")>();
  return {
    ...actual,
    // The real writers both go to the store over IPC; what this file owns is *which*
    // model the ring picks, so the write itself is a probe.
    setVibeCellModel: h.setters.model,
    setVibeCellThinkingLevel: h.setters.thinking,
  };
});

vi.mock("../agent/AgentThread", () => ({
  AgentThread: (props: {
    activeApproval?: { id: string } | null;
    chrome?: string;
    composerControls?: string;
    onApprovalDecision?: (approval: { id: string }, status: "approved" | "rejected") => Promise<void>;
    onLeaveInput?: () => void;
    onQueue?: (payload: { attachments: unknown[]; content: string }) => boolean;
    thread: { id: string };
  }) => (
    <div
      data-chrome={props.chrome ?? "full"}
      data-composer-controls={props.composerControls ?? ""}
      data-thread={props.thread.id}
    >
      {/* What the real composer does with `onQueue`: hand the sentence over, and it
          answers whether the box may be cleared. */}
      {props.onQueue
        ? (
            <button
              data-queue-probe=""
              onClick={() => {
                const taken = props.onQueue!({ attachments: [], content: "held sentence" });
                h.probe.taken = taken;
              }}
              type="button"
            />
          )
        : null}
      {/* The real `AgentThread` is what answers an approval and what reports that the
          editor was left; the panel only supplies the two callbacks. */}
      {props.onApprovalDecision
        ? (
            <button
              data-approval-probe=""
              onClick={() => {
                void props.onApprovalDecision?.(props.activeApproval ?? { id: "approval" }, "approved");
              }}
              type="button"
            />
          )
        : null}
      {props.onLeaveInput
        ? <button data-leave-input-probe="" onClick={() => props.onLeaveInput!()} type="button" />
        : null}
      {props.chrome === "input"
        ? <div contentEditable data-input-editor="" suppressContentEditableWarning />
        : null}
      {/* The real view's transcript attribute. The panel addresses a cell's transcript by
          it, so a stub without one would make every scroll test pass on nothing. */}
      <div data-chat-scroll="" />
    </div>
  ),
}));

vi.mock("../../components/layout/ContextPanel", () => ({
  ContextPanel: (props: { onToggleExpanded?: () => void }) => (
    <div data-vibe-column-stub="">
      <button data-toggle-expanded="" onClick={props.onToggleExpanded} type="button" />
    </div>
  ),
}));

vi.mock("../remote-peer/RemoteFilesPanel", () => ({
  RemoteFilesPanel: (props: { onToggleExpanded?: () => void }) => (
    <div data-vibe-column-stub="">
      <button data-toggle-expanded="" onClick={props.onToggleExpanded} type="button" />
    </div>
  ),
}));

vi.mock("../../components/layout/hooks/useApprovals", () => ({
  useApprovals: () => h.approvals,
}));

vi.mock("../../components/layout/hooks/usePendingApprovalCounts", () => ({
  usePendingApprovalCounts: () => h.counts,
}));

vi.mock("../../integrations/agent/agentStateCache", () => ({
  useCachedAgentState: () => h.cached.current,
}));

/**
 * The window API as the panel uses it: `useIsFullscreen` reads `isFullscreen` and re-reads
 * on `onResized`, and the bar/menu/command call `setFullscreen`.
 *
 * **One shared object**, not a fresh one per `getCurrentWindow()`: the panel asks for the
 * window at the moment it needs it, so a test that wants to make a call fail has to be
 * able to reach the same instance the panel will get.
 */
vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => h.win,
}));

const VIEW: VibeThreadViewProps = {
  agentConnection: { status: "connected" } as VibeThreadViewProps["agentConnection"],
  approvalTier: "off",
  futureBalance: null,
  futureSessionStatus: "ready",
  loadingStore: false,
  modelId: "future/x",
  modelOptions: [],
  onChangeApprovalTier: () => {},
  onApprovalDecided: () => {},
  onForked: () => {},
  onModelChange: () => {},
  onOpenAccount: () => {},
  onOpenModels: () => {},
  onOpenProviders: () => {},
  onRetryAgentConnection: () => {},
  onThinkingLevelChange: () => {},
  onThreadActivity: () => {},
  sessionMentions: [],
  skillRecommend: false,
  thinkingLevel: "medium",
};

const WORKSPACES: StoredWorkspace[] = [];

function thread(id: string, createdAt: number, overrides: Partial<StoredThread> = {}): StoredThread {
  return {
    agentSessionId: `session-${id}`,
    createdAt,
    id,
    mode: "chat",
    pinned: true,
    readonly: false,
    status: "active",
    title: `Conversation ${id}`,
    updatedAt: createdAt,
    workspaceId: "ws",
    ...overrides,
  };
}

let container: HTMLDivElement;
let root: ReturnType<typeof createRoot>;
let uninstallShortcuts: () => void;

/** Re-render the mounted panel with new inputs — what a run-status push does. */
function rerender(options: { runStatuses?: Record<string, { status: string }>; threads: StoredThread[] }) {
  act(() => {
    root.render(
      <VibeStage
        catalogs={[]}
        onClose={() => {}}
        onOpenInShell={() => {}}
        onNewConversation={() => {}}
        onOpenRemoteInShell={() => {}}
        onPinLocal={() => {}}
        onPinRemote={() => {}}
        peers={[]}
        runStatuses={(options.runStatuses ?? {}) as never}
        threads={options.threads}
        unreadThreadIds={new Set()}
        view={VIEW}
        workspaces={WORKSPACES}
      />,
    );
  });
}

function mount(options: {
  threads: StoredThread[];
  catalogs?: RemoteCatalog[];
  peers?: RemotePeer[];
  runStatuses?: Record<string, { status: string }>;
  unread?: string[];
  onClose?: () => void;
  onOpenInShell?: (thread: StoredThread) => void;
  onOpenRemoteInShell?: (desktopId: string, sessionId: string) => void;
  onNewConversation?: () => void;
  onPinLocal?: (threadId: string, pinned: boolean) => void;
  onPinRemote?: (desktopId: string, sessionId: string, pinned: boolean) => void;
  view?: Partial<VibeThreadViewProps>;
  workspaces?: StoredWorkspace[];
}) {
  act(() => {
    root.render(
      <VibeStage
        catalogs={options.catalogs ?? []}
        onClose={options.onClose ?? (() => {})}
        onOpenInShell={options.onOpenInShell ?? (() => {})}
        onNewConversation={options.onNewConversation ?? (() => {})}
        onOpenRemoteInShell={options.onOpenRemoteInShell ?? (() => {})}
        onPinLocal={options.onPinLocal ?? (() => {})}
        onPinRemote={options.onPinRemote ?? (() => {})}
        peers={options.peers ?? []}
        runStatuses={(options.runStatuses ?? {}) as never}
        threads={options.threads}
        unreadThreadIds={new Set(options.unread ?? [])}
        view={{ ...VIEW, ...options.view }}
        workspaces={options.workspaces ?? WORKSPACES}
      />,
    );
  });
}

function press(init: KeyboardEventInit) {
  act(() => {
    document.body.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, cancelable: true, ...init }));
  });
}

const chromes = () => [...container.querySelectorAll<HTMLElement>("[data-chrome]")];
function drawn(chrome: string) {
  return chromes()
    .filter(element => element.dataset.chrome === chrome)
    .map(element => element.dataset.thread);
}
const editableCount = () => container.querySelectorAll("[contenteditable=\"true\"]").length;

/** The row the group menu is on, by id. */
const selectedRow = () => container.querySelector("[aria-current=\"true\"]")?.getAttribute("data-vibe-menu-row") ?? null;

/** Walk the menu down to a row by id, however many presses that takes. */
function walkTo(id: string) {
  for (let step = 0; step < 12 && selectedRow() !== id; step++)
    press({ code: "ArrowDown", key: "ArrowDown" });
  expect(selectedRow(), id).toBe(id);
}

beforeEach(() => {
  clearVibePreferences();
  // The queue hands a sentence back by writing it into the conversation's composer
  // draft, so a draft left by one case is state the next case would see.
  sessionStorage.clear();
  resetAllCommandBindings();
  resetShortcutScopes();
  clearCommandHandlers();
  h.counts = new Map();
  h.win.setFullscreen.mockClear();
  h.cached.current = null;
  container = document.body.appendChild(document.createElement("div"));
  root = createRoot(container);
  uninstallShortcuts = installShortcuts();
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  uninstallShortcuts();
  localStorage.clear();
  sessionStorage.clear();
});

describe("the panel's layers", () => {
  it("draws one readable cell per pinned conversation, and no editor", () => {
    mount({ threads: [thread("a", 100), thread("b", 200), thread("c", 300)] });
    expect(drawn("read")).toEqual(["a", "b", "c"]);
    // Nothing to dictate into until the input layer is opened: the panel does not
    // scatter composers across its cells.
    expect(editableCount()).toBe(0);
  });

  it("opens the focused conversation's input layer on the panel's own key", () => {
    mount({ threads: [thread("a", 100), thread("b", 200)] });
    press({ code: "Enter", key: "Enter" });
    expect(drawn("input")).toEqual(["a"]);
    // The invariant: exactly one place a dictation can land.
    expect(editableCount()).toBe(1);
  });

  it("keeps the rest of the panel visible, dimmed, behind the input layer", () => {
    // A person talking to one conversation still needs to see who is running and
    // who is waiting: the layer is a card over the grid, not a screen of its own.
    // The focused conversation is drawn by the card itself — never both — because
    // two mounts would mean two subscriptions and two history reads for one
    // conversation.
    mount({ threads: [thread("a", 100), thread("b", 200), thread("c", 300)] });
    press({ code: "Enter", key: "Enter" });
    expect(drawn("input")).toEqual(["a"]);
    expect(drawn("read")).toEqual(["b", "c"]);
    expect(editableCount()).toBe(1);
  });

  it("keeps exactly one editor when the focus moves between conversations", () => {
    mount({ threads: [thread("a", 100), thread("b", 200), thread("c", 300)] });
    press({ code: "Enter", key: "Enter" });
    expect(drawn("input")).toEqual(["a"]);
    // Out to reading, right to the next conversation, back into the input layer.
    press({ code: "Escape", key: "Escape" });
    expect(editableCount()).toBe(0);
    press({ code: "ArrowRight", key: "ArrowRight" });
    press({ code: "Enter", key: "Enter" });
    expect(drawn("input")).toEqual(["b"]);
    expect(editableCount()).toBe(1);
  });

  it("closes the panel from the grid's back key", () => {
    const onClose = vi.fn();
    mount({ onClose, threads: [thread("a", 100)] });
    press({ code: "Escape", key: "Escape" });
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("walks back one layer at a time", () => {
    const onClose = vi.fn();
    mount({ onClose, threads: [thread("a", 100), thread("b", 200)] });
    press({ code: "Enter", key: "Enter" });
    expect(drawn("input")).toEqual(["a"]);
    // Input → reading: nothing closes yet.
    press({ code: "Escape", key: "Escape" });
    expect(onClose).not.toHaveBeenCalled();
    expect(editableCount()).toBe(0);
    // Reading → grid: still nothing.
    press({ code: "Escape", key: "Escape" });
    expect(onClose).not.toHaveBeenCalled();
    // Grid → closed.
    press({ code: "Escape", key: "Escape" });
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("moves the ring with the direction keys", () => {
    mount({ threads: [thread("a", 100), thread("b", 200), thread("c", 300), thread("d", 400)] });
    // Four cells in a 2×2: down from the first column is the third cell.
    press({ code: "ArrowDown", key: "ArrowDown" });
    press({ code: "Enter", key: "Enter" });
    expect(drawn("input")).toEqual(["c"]);
  });

  it("jumps to a numbered cell", () => {
    mount({ threads: [thread("a", 100), thread("b", 200), thread("c", 300), thread("d", 400)] });
    press({ code: "Digit3", key: "3" });
    press({ code: "Enter", key: "Enter" });
    expect(drawn("input")).toEqual(["c"]);
  });

  it("switches conversations with the side keys while reading", () => {
    mount({ threads: [thread("a", 100), thread("b", 200), thread("c", 300)] });
    press({ code: "Enter", key: "Enter" });
    press({ code: "Escape", key: "Escape" });
    press({ code: "ArrowRight", key: "ArrowRight" });
    press({ code: "ArrowRight", key: "ArrowRight" });
    press({ code: "Enter", key: "Enter" });
    expect(drawn("input")).toEqual(["c"]);
  });

  it("does not let the grid's keys reach over a cell's editor", () => {
    // A bare key is the text field's once the caret is in one. The input layer's
    // own key is the exception, and it is registered by the composer.
    mount({ threads: [thread("a", 100), thread("b", 200)] });
    press({ code: "Enter", key: "Enter" });
    expect(drawn("input")).toEqual(["a"]);
    expect(editableCount()).toBe(1);
  });
});

describe("the menu", () => {
  it("opens on the remote's menu button and closes on it again", () => {
    mount({ threads: [thread("a", 100), thread("b", 200)] });
    press({ code: "ContextMenu", key: "ContextMenu" });
    expect(container.querySelector("[role=\"menu\"]")).not.toBeNull();
    press({ code: "ContextMenu", key: "ContextMenu" });
    expect(container.querySelector("[role=\"menu\"]")).toBeNull();
  });

  it("holds the panel's chords while it is up", () => {
    // `useKeyboardOwner`: a chord that would move a cell must not fire behind the
    // menu. Without the registration the dispatcher would still see `⌘⌥→` and the
    // order would move under a sheet the reader is reading.
    mount({ threads: [thread("a", 100), thread("b", 200)] });
    press({ code: "ContextMenu", key: "ContextMenu" });
    press({ code: "ArrowRight", key: "ArrowRight", ctrlKey: true, altKey: true });
    expect(drawn("read")).toEqual(["a", "b"]);
    expect(readVibePreferences().order).toEqual([]);
  });

  it("keeps the grid's arrows when the menu is up", () => {
    // The menu holds the keyboard (`useKeyboardOwner`), so `↓` walks its rows
    // instead of moving the ring behind it. Without that the reader would open the
    // menu and silently re-select a different conversation.
    mount({ threads: [thread("a", 100), thread("b", 200), thread("c", 300)] });
    press({ code: "ContextMenu", key: "ContextMenu" });
    press({ code: "ArrowDown", key: "ArrowDown" });
    press({ code: "Escape", key: "Escape" });
    press({ code: "Enter", key: "Enter" });
    expect(drawn("input")).toEqual(["a"]);
  });

  it("changes the layout from its own row and remembers it", () => {
    mount({ threads: [thread("a", 100), thread("b", 200), thread("c", 300)] });
    press({ code: "ContextMenu", key: "ContextMenu" });
    walkTo("layout");
    // → moves it off "automatic".
    press({ code: "ArrowRight", key: "ArrowRight" });
    expect(readVibePreferences().columns).toBe(1);
    // …and the panel says so, so the reader can see what they just chose: three
    // cells in one column, marked as a manual choice.
    expect(container.textContent).toContain("1×3");
  });

  it("unpins the focused cell from the menu", () => {
    const onPinLocal = vi.fn();
    mount({ onPinLocal, threads: [thread("a", 100), thread("b", 200)] });
    press({ code: "ContextMenu", key: "ContextMenu" });
    // Walk to "remove". The walk skips the row that cannot run (the focused cell
    // is already first, so "move earlier" is disabled), which is why this counts
    // steps by what is selected rather than by a fixed number of presses.
    const selected = () => container.querySelector("[aria-current=\"true\"]")?.getAttribute("data-vibe-menu-row");
    for (let step = 0; step < 8 && selected() !== "remove"; step++)
      press({ code: "ArrowDown", key: "ArrowDown" });
    expect(selected()).toBe("remove");
    press({ code: "Enter", key: "Enter" });
    expect(onPinLocal).toHaveBeenCalledWith("a", false);
  });

  it("gives the caret back when it closes from the input layer", () => {
    // The menu takes focus when it opens. Closing it while dictating must hand the
    // caret back, or the next `⏎` sends nothing and the next sentence is lost.
    const focused = vi.fn();
    const off = onFutureEvent("focus-composer", focused);
    try {
      mount({ threads: [thread("a", 100)] });
      press({ code: "Enter", key: "Enter" });
      expect(editableCount()).toBe(1);
      press({ code: "ContextMenu", key: "ContextMenu" });
      press({ code: "Escape", key: "Escape" });
      expect(focused).toHaveBeenCalledTimes(1);
      expect(editableCount()).toBe(1);
    }
    finally {
      off();
    }
  });

  it("does not ask for a caret when it closes from the grid", () => {
    const focused = vi.fn();
    const off = onFutureEvent("focus-composer", focused);
    try {
      mount({ threads: [thread("a", 100)] });
      press({ code: "ContextMenu", key: "ContextMenu" });
      press({ code: "Escape", key: "Escape" });
      expect(focused).not.toHaveBeenCalled();
    }
    finally {
      off();
    }
  });
});

describe("the attention queue", () => {
  // `h.counts` is the mocked global approval counts; `runStatuses` is passed in.
  function mountWith(runStatuses: Record<string, { status: string }>, counts: Record<string, number> = {}) {
    h.counts = new Map(Object.entries(counts));
    mount({ runStatuses, threads: [thread("a", 100), thread("b", 200), thread("c", 300)] });
  }

  it("jumps to the next cell that needs a human, and lands on its reading layer", () => {
    // The whole point: "what does this want from me" is answered by opening it, not
    // by moving a ring and making the reader press again.
    mountWith({ b: { status: "failed" } });
    press({ code: "KeyA", key: "a", ctrlKey: true, shiftKey: true });
    expect(drawn("read")).toEqual(["b"]);
  });

  it("walks the queue in panel order and wraps", () => {
    mountWith({ b: { status: "failed" }, c: { status: "waiting_approval" } });
    press({ code: "KeyA", key: "a", ctrlKey: true, shiftKey: true });
    expect(drawn("read")).toEqual(["b"]);
    press({ code: "KeyA", key: "a", ctrlKey: true, shiftKey: true });
    expect(drawn("read")).toEqual(["c"]);
    press({ code: "KeyA", key: "a", ctrlKey: true, shiftKey: true });
    expect(drawn("read")).toEqual(["b"]);
  });

  it("counts a pending approval and an unseen finish as attention", () => {
    mountWith({ a: { status: "running" } }, { a: 1 });
    press({ code: "KeyA", key: "a", ctrlKey: true, shiftKey: true });
    expect(drawn("read")).toEqual(["a"]);
  });

  it("leaves a busy conversation out of the queue", () => {
    // "running" is not "needs you": a queue that included it would be every cell,
    // and then the key would be a slower version of the arrow keys.
    mountWith({ a: { status: "running" }, b: { status: "queued" }, c: { status: "running" } });
    press({ code: "KeyA", key: "a", ctrlKey: true, shiftKey: true });
    // Nothing waiting means the key is unclaimed, so the panel is still on the grid.
    expect(drawn("read")).toEqual(["a", "b", "c"]);
  });

  it("says it is waiting in the panel's own bar and on the cell", () => {
    mountWith({ b: { status: "failed" } });
    expect(container.querySelector("[data-vibe-attention]")?.textContent).toContain("1");
    expect(container.querySelector("[data-vibe-attention-reason=\"failed\"]")).not.toBeNull();
    // The bar and the marker answer the same question from two places: "somewhere in
    // here" and "this one".
    mountWith({ a: { status: "running" } });
    expect(container.querySelector("[data-vibe-attention]")).toBeNull();
  });

  it("lists what is waiting in the menu, and says so when nothing is", () => {
    mountWith({ b: { status: "failed" } });
    press({ code: "ContextMenu", key: "ContextMenu" });
    expect(selectedRow()).toBe("attention:local:b");
    press({ code: "Escape", key: "Escape" });
    mountWith({});
    press({ code: "ContextMenu", key: "ContextMenu" });
    // A disabled row, not an absent one: "nothing is waiting" is the answer to the
    // question the menu was opened to ask.
    expect(container.querySelector("[data-vibe-menu-row=\"attention:none\"]")?.getAttribute("aria-disabled")).toBe("true");
  });

  it("opens the waiting conversation from the menu", () => {
    mountWith({ c: { status: "waiting_approval" } });
    press({ code: "ContextMenu", key: "ContextMenu" });
    press({ code: "Enter", key: "Enter" });
    expect(drawn("read")).toEqual(["c"]);
    expect(container.querySelector("[role=\"menu\"]")).toBeNull();
  });
});

describe("other machines", () => {
  const catalog = (desktopId: string, sessions: Array<Record<string, unknown>>) =>
    ({ desktopId, sessions } as unknown as RemoteCatalog);
  const peer = (desktopId: string, name: string) => ({ desktopId, name } as RemotePeer);

  function mountMixed(over: Record<string, unknown> = {}) {
    // The `status` on a host's row is its last run's, not a lifecycle
    // (`remote_host::catalog`); a finished run is the ordinary state of a pinned
    // conversation, so these rows carry one.
    mount({
      catalogs: [
        catalog("desk_1", [
          { pinned: true, sessionId: "s1", status: "completed", title: "Remote one" },
          { pinned: false, sessionId: "s2", status: "completed", title: "Remote unpinned" },
        ]),
      ],
      peers: [peer("desk_1", "Studio")],
      threads: [thread("a", 100)],
      ...over,
    });
  }

  it("puts a host's pinned conversations in the grid beside this machine's", () => {
    mountMixed();
    expect(container.querySelectorAll("[data-vibe-cell]")).toHaveLength(2);
    expect(container.querySelector("[data-vibe-cell-kind=\"remote\"]")?.textContent).toContain("Remote one");
    // A cell says which machine it is on; a grid of one host's conversations that
    // looked like this machine's would be worse than no grid.
    expect(container.textContent).toContain("Studio");
  });

  it("filters to one machine from the menu and remembers it", () => {
    mountMixed();
    press({ code: "ContextMenu", key: "ContextMenu" });
    // attention (none) → add → device; ←/→ walks "all" → this machine → each host,
    // in the order the machines are paired.
    walkTo("device");
    press({ code: "ArrowRight", key: "ArrowRight" });
    expect(container.querySelector("[data-vibe-device]")?.textContent).toBe("This Mac");
    expect(container.querySelectorAll("[data-vibe-cell]")).toHaveLength(1);
    expect(container.querySelector("[data-vibe-cell-kind=\"local\"]")).not.toBeNull();
    press({ code: "ArrowRight", key: "ArrowRight" });
    expect(container.querySelector("[data-vibe-device]")?.textContent).toBe("Studio");
    expect(readVibePreferences().device).toEqual({ desktopId: "desk_1", kind: "device" });
    // Only the host's cell is left.
    expect(container.querySelectorAll("[data-vibe-cell]")).toHaveLength(1);
    expect(container.querySelector("[data-vibe-cell-kind=\"remote\"]")).not.toBeNull();
  });

  it("starts from the stored filter", () => {
    writeVibeDevice({ desktopId: "desk_1", kind: "device" });
    mountMixed();
    expect(container.querySelectorAll("[data-vibe-cell]")).toHaveLength(1);
  });

  it("opens a host's conversation in the main window, never in an input layer", () => {
    // Dictation is this machine's: a remote cell has no composer at all, so `OK` on
    // one must go to its reading layer instead of opening an editor that would not
    // exist (`VIBE.md` §5).
    const onOpenRemoteInShell = vi.fn();
    mountMixed({ onOpenRemoteInShell });
    expect(drawn("input")).toEqual([]);
    // The host's cell is second in the order (undated members sort last).
    press({ code: "Digit2", key: "2" });
    press({ code: "Enter", key: "Enter" });
    expect(drawn("input")).toEqual([]);
    expect(readVibePreferences().order).toEqual([]);
    // It is in the reading layer instead.
    expect(container.getAttribute).toBeTruthy();
    expect(container.querySelector("[data-vibe-layer]")?.getAttribute("data-vibe-layer")).toBe("read");
    // …and the key that would open an input layer is not offered there.
    expect(container.textContent).not.toContain("进入输入层");
  });

  it("unpins a host's conversation on the machine that owns it", () => {
    const onPinRemote = vi.fn();
    const onPinLocal = vi.fn();
    mountMixed({ onPinLocal, onPinRemote });
    press({ code: "Digit2", key: "2" });
    press({ code: "Backspace", key: "Backspace", ctrlKey: true, shiftKey: true });
    expect(onPinRemote).toHaveBeenCalledWith("desk_1", "s1", false);
    expect(onPinLocal).not.toHaveBeenCalled();
  });
});

describe("adding a conversation", () => {
  const catalog = (desktopId: string, sessions: Array<Record<string, unknown>>) =>
    ({ desktopId, sessions } as unknown as RemoteCatalog);

  function mountForAdd(onPinLocal: (id: string, pinned: boolean) => void, onPinRemote: (desktopId: string, sessionId: string, pinned: boolean) => void) {
    mount({
      catalogs: [catalog("desk_1", [
        { pinned: false, sessionId: "s1", status: "completed", title: "Remote candidate" },
      ])],
      onPinLocal,
      onPinRemote,
      peers: [{ desktopId: "desk_1", name: "Studio" } as RemotePeer],
      threads: [thread("a", 100), thread("candidate", 200, { pinned: false, title: "Local candidate" })],
    });
  }

  function openAdd() {
    press({ code: "ContextMenu", key: "ContextMenu" });
    walkTo("add");
    press({ code: "Enter", key: "Enter" });
  }

  it("lists a machine's unpinned conversations and adds one by pinning it", () => {
    const onPinLocal = vi.fn();
    mountForAdd(onPinLocal, vi.fn());
    openAdd();
    const row = container.querySelector("[data-add-choice=\"local:candidate\"]");
    expect(row).not.toBeNull();
    act(() => {
      row!.querySelector("button")!.click();
    });
    expect(onPinLocal).toHaveBeenCalledWith("candidate", true);
  });

  it("adds a host's conversation on that host", () => {
    const onPinRemote = vi.fn();
    mountForAdd(vi.fn(), onPinRemote);
    openAdd();
    // Machine → Studio, then the conversation.
    act(() => {
      container.querySelector<HTMLElement>("[data-add-machine=\"desk_1\"]")!.click();
    });
    const row = container.querySelector("[data-add-choice=\"remote:desk_1:s1\"]");
    expect(row).not.toBeNull();
    act(() => {
      row!.querySelector("button")!.click();
    });
    expect(onPinRemote).toHaveBeenCalledWith("desk_1", "s1", true);
  });

  it("does not offer a conversation that is already in the panel", () => {
    const threads = [thread("a", 100)];
    mount({
      catalogs: [catalog("desk_1", [{ pinned: true, sessionId: "s1", status: "completed", title: "Already" }])],
      threads,
    });
    openAdd();
    act(() => {
      container.querySelector<HTMLElement>("[data-add-machine=\"desk_1\"]")!.click();
    });
    // Nothing to add on that machine, and the dialog says so rather than showing an
    // empty list.
    expect(container.querySelector("[data-add-choice=\"remote:desk_1:s1\"]")).toBeNull();
  });
});

describe("this cell's settings", () => {
  // What used to be four dropdowns inside a dictated sentence, now a read-only line
  // plus three menu rows (§5.6). The line is the part that matters most: a reader
  // watching five cells has to be able to answer "what is this one running" without
  // changing it.
  const MODEL_OPTIONS = [
    { id: "deepseek-v4-pro", label: "DeepSeek V4 Pro", provider: "future" },
    { id: "claude-sonnet", label: "Claude Sonnet", provider: "future" },
  ] as never;

  function mountWithSettings(over: Record<string, unknown> = {}) {
    h.cached.current = { model: "future/deepseek-v4-pro", thinkingLevel: "medium" };
    mount({
      threads: [thread("a", 100)],
      ...over,
    });
    act(() => {
      root.render(
        <VibeStage
          catalogs={[]}
          onClose={() => {}}
          onOpenInShell={() => {}}
          onNewConversation={() => {}}
          onOpenRemoteInShell={() => {}}
          onPinLocal={() => {}}
          onPinRemote={() => {}}
          peers={[]}
          runStatuses={{}}
          threads={[thread("a", 100)]}
          unreadThreadIds={new Set()}
          view={{ ...VIEW, approvalTier: "off", modelOptions: MODEL_OPTIONS }}
          workspaces={WORKSPACES}
        />,
      );
    });
  }

  it("shows the model, thinking level and approval tier on the reading layer", () => {
    mountWithSettings();
    press({ code: "Enter", key: "Enter" });
    press({ code: "Escape", key: "Escape" });
    const line = container.querySelector("[data-vibe-settings]");
    expect(line).not.toBeNull();
    expect(line!.textContent).toContain("DeepSeek V4 Pro");
    // The approval tier is named, because "what is allowed to run here" is the other
    // half of the question a reader asks of a cell. Asserted by attribute: the label is
    // translated, the tier is not.
    expect(line!.getAttribute("data-vibe-approval")).toBe("off");
  });

  it("shows the same line on the input layer", () => {
    mountWithSettings();
    press({ code: "Enter", key: "Enter" });
    const lines = [...container.querySelectorAll("[data-vibe-settings]")];
    expect(lines).toHaveLength(1);
    expect(lines[0]!.textContent).toContain("DeepSeek V4 Pro");
  });

  it("asks the input layer's composer for values only, and leaves the cells as they are", () => {
    // The whole point of §5.6: the four dropdowns move into the menu, and the layer
    // keeps one read-only line. `AgentThread` is stubbed, so this asserts the prop the
    // panel hands it; `Composer.inputLayer.test.tsx` holds the composer's side.
    mountWithSettings();
    expect(container.querySelector("[data-chrome=\"read\"]")!.getAttribute("data-composer-controls")).toBe("");
    press({ code: "Enter", key: "Enter" });
    expect(container.querySelector("[data-chrome=\"input\"]")!.getAttribute("data-composer-controls")).toBe("none");
  });

  it("offers the three values in the menu, adjustable with ← and →", () => {
    const onChangeApprovalTier = vi.fn();
    h.cached.current = { model: "future/deepseek-v4-pro", thinkingLevel: "medium" };
    act(() => {
      root.render(
        <VibeStage
          catalogs={[]}
          onClose={() => {}}
          onOpenInShell={() => {}}
          onNewConversation={() => {}}
          onOpenRemoteInShell={() => {}}
          onPinLocal={() => {}}
          onPinRemote={() => {}}
          peers={[]}
          runStatuses={{}}
          threads={[thread("a", 100)]}
          unreadThreadIds={new Set()}
          view={{ ...VIEW, approvalTier: "off", modelOptions: MODEL_OPTIONS, onChangeApprovalTier }}
          workspaces={WORKSPACES}
        />,
      );
    });
    press({ code: "ContextMenu", key: "ContextMenu" });
    // Model, thinking, approval: the three rows that used to be the composer's.
    for (const id of ["model", "thinking", "approval"])
      expect(container.querySelector(`[data-vibe-menu-row="${id}"]`), id).not.toBeNull();
    walkTo("approval");
    // The approval tier is application-wide, so its row drives the shell's setter — the
    // same one the main window's dropdown uses.
    press({ code: "ArrowRight", key: "ArrowRight" });
    expect(onChangeApprovalTier).toHaveBeenCalledWith("manual");
  });

  it("keeps the model row out of a host's conversation", () => {
    // A host's model is that machine's; the panel changes what it owns.
    mount({
      catalogs: [{ desktopId: "desk_1", sessions: [{ pinned: true, sessionId: "s1", status: "completed", title: "Remote" }] } as never],
      peers: [{ desktopId: "desk_1", name: "Studio" } as never],
      threads: [],
    });
    press({ code: "ContextMenu", key: "ContextMenu" });
    expect(container.querySelector("[data-vibe-menu-row=\"model\"]")).toBeNull();
    expect(container.querySelector("[data-vibe-menu-row=\"approval\"]")).toBeNull();
  });
});

describe("the send queue", () => {
  // The five rules the queue was commissioned under, driven from the panel's side:
  // visible, cancellable without losing the text, one per conversation, never sent by
  // itself, kept across a failure. The pure half is `vibeQueue.test.ts`; here it is the
  // cell markers, the menu row and the key.
  function handOver() {
    const probe = container.querySelector<HTMLElement>("[data-queue-probe]");
    expect(probe, "a composer that can hand a sentence over").not.toBeNull();
    act(() => probe!.click());
    return h.probe.taken;
  }

  it("marks the cell as soon as a sentence is held", () => {
    mount({ runStatuses: { a: { status: "running" } }, threads: [thread("a", 100)] });
    expect(container.querySelector("[data-vibe-queued]")).toBeNull();
    expect(handOver()).toBe(true);
    expect(container.querySelector("[data-vibe-queued]")).not.toBeNull();
  });

  it("refuses a second one, so a held sentence is never overwritten", () => {
    mount({ runStatuses: { a: { status: "running" } }, threads: [thread("a", 100)] });
    expect(handOver()).toBe(true);
    // The refusal is what keeps the composer's own text in the box (see
    // `Composer.inputLayer.test.tsx`); here it only has to say "no".
    expect(handOver()).toBe(false);
  });

  it("hands the sentence back when the run ends, and never sends it", () => {
    mount({ runStatuses: { a: { status: "running" } }, threads: [thread("a", 100)] });
    handOver();
    // The run finishes. The panel now puts the text back in the box and says so — it
    // does not send, which is why there is no send path anywhere in `vibeQueue.ts`.
    rerender({ runStatuses: { a: { status: "completed" } }, threads: [thread("a", 100)] });
    expect(container.querySelector("[data-vibe-queued]")).toBeNull();
    expect(container.querySelector("[data-vibe-returned]")).not.toBeNull();
    expect(loadComposerDraft("a")?.text).toBe("held sentence");
  });

  it("keeps the sentence across a failed run", () => {
    // A failure is not a reason to lose it and not a reason to re-send it: it comes
    // back to the box like any other ending.
    mount({ runStatuses: { a: { status: "running" } }, threads: [thread("a", 100)] });
    handOver();
    rerender({ runStatuses: { a: { status: "failed" } }, threads: [thread("a", 100)] });
    expect(loadComposerDraft("a")?.text).toBe("held sentence");
    expect(container.querySelector("[data-vibe-returned]")).not.toBeNull();
  });

  it("offers the cancel row only while something is held, and puts the text back", () => {
    mount({ runStatuses: { a: { status: "running" } }, threads: [thread("a", 100)] });
    press({ code: "ContextMenu", key: "ContextMenu" });
    expect(container.querySelector("[data-vibe-menu-row=\"queue-cancel\"]")).toBeNull();
    press({ code: "Escape", key: "Escape" });

    handOver();
    press({ code: "ContextMenu", key: "ContextMenu" });
    expect(container.querySelector("[data-vibe-menu-row=\"queue-cancel\"]")).not.toBeNull();
    walkTo("queue-cancel");
    press({ code: "Enter", key: "Enter" });
    // Cancelling is "not now": the sentence is in the box, not gone.
    expect(loadComposerDraft("a")?.text).toBe("held sentence");
    expect(container.querySelector("[data-vibe-queued]")).toBeNull();
  });

  it("cancels from the input layer's own key", () => {
    mount({ runStatuses: { a: { status: "running" } }, threads: [thread("a", 100)] });
    handOver();
    // Into the input layer: the key belongs to that scope (the caret is in the editor).
    press({ code: "Enter", key: "Enter" });
    // `mod+shift+x`: `meta` on the platform the test runs as (the dispatcher reads
    // `mod` from `isMacOS`), and `ctrl` elsewhere.
    press({ code: "KeyX", key: "x", metaKey: isMacOS, ctrlKey: !isMacOS, shiftKey: true });
    expect(loadComposerDraft("a")?.text).toBe("held sentence");
    expect(container.querySelector("[data-vibe-queued]")).toBeNull();
  });

  it("stops marking a handed-back sentence once the reader opens it", () => {
    mount({ runStatuses: { a: { status: "running" } }, threads: [thread("a", 100)] });
    handOver();
    rerender({ runStatuses: { a: { status: "completed" } }, threads: [thread("a", 100)] });
    expect(container.querySelector("[data-vibe-returned]")).not.toBeNull();
    // Opening the conversation's input layer is what "I have seen it" means: the text
    // is in the box in front of them.
    press({ code: "Enter", key: "Enter" });
    expect(container.querySelector("[data-vibe-returned]")).toBeNull();
  });

  it("does not queue for a host's conversation", () => {
    // A remote cell has no composer at all, so there is nothing to hand over — the
    // panel offers no queue for it rather than a queue nothing can fill.
    mount({
      catalogs: [{ desktopId: "desk_1", sessions: [{ pinned: true, sessionId: "s1", status: "completed", title: "Remote" }] } as never],
      peers: [{ desktopId: "desk_1", name: "Studio" } as never],
      threads: [],
    });
    expect(container.querySelector("[data-queue-probe]")).toBeNull();
  });
});

describe("the letters, beside the arrows", () => {
  // `h`/`j`/`k`/`l` on the same commands as the arrows: same movement, second key. What
  // matters is that they move the ring in the grid, switch conversation while reading, and
  // never fire over an editor — the last one is `dispatch`'s caret rule, and it is the
  // reason a letter may sit next to a bare arrow at all.
  it("moves the ring with the letters", () => {
    mount({ threads: [thread("a", 100), thread("b", 200), thread("c", 300), thread("d", 400)] });
    // 2×2: `l` right, `j` down.
    press({ code: "KeyL", key: "l" });
    press({ code: "KeyJ", key: "j" });
    press({ code: "Enter", key: "Enter" });
    expect(drawn("input")).toEqual(["d"]);
  });

  it("switches conversation with h and l while reading", () => {
    mount({ threads: [thread("a", 100), thread("b", 200), thread("c", 300)] });
    press({ code: "Enter", key: "Enter" });
    press({ code: "Escape", key: "Escape" });
    press({ code: "KeyL", key: "l" });
    press({ code: "KeyL", key: "l" });
    press({ code: "KeyH", key: "h" });
    press({ code: "Enter", key: "Enter" });
    // a → b → c → b.
    expect(drawn("input")).toEqual(["b"]);
  });

  it("leaves the letters to the editor while dictating", () => {
    // Typing `h` in a dictation must be an `h`, and this is the rule that allows it:
    // a modifier-less key belongs to the field that has the caret.
    mount({ threads: [thread("a", 100), thread("b", 200)] });
    press({ code: "Enter", key: "Enter" });
    const before = container.querySelector("[data-vibe-layer]")?.getAttribute("data-vibe-layer");
    // The caret is in the editor here; the layer must not change.
    press({ code: "KeyL", key: "l" });
    expect(container.querySelector("[data-vibe-layer]")?.getAttribute("data-vibe-layer")).toBe(before);
    expect(drawn("input")).toEqual(["a"]);
  });
});

describe("the reading layer's second column", () => {
  // Three states, per conversation, remembered by the panel's own key. The column itself
  // is the shell's existing `ContextPanel` (stubbed here): what this file owns is whether
  // it is there, and what the keys and the menu row do to that answer.
  function mountWithPanel(over: Record<string, unknown> = {}) {
    mount({ threads: [thread("a", 100), thread("b", 200)], ...over });
  }

  /** The stub: `ContextPanel` is the shell's, and mounting it for real here would test it. */
  const column = () => container.querySelector("[data-vibe-column-stub]");

  it("shows it automatically on a wide window, and not on a narrow one", () => {
    // jsdom reports 1024px by default, i.e. below PANEL_AUTO_WIDTH.
    mountWithPanel();
    press({ code: "Enter", key: "Enter" });
    press({ code: "Escape", key: "Escape" });
    expect(column()).toBeNull();
  });

  it("hides it with the panel's own key, and comes back to automatic", () => {
    // `⌘\` twice: hidden, then back to automatic — not back to pinned.
    mountWithPanel();
    press({ code: "Enter", key: "Enter" });
    press({ code: "Escape", key: "Escape" });
    press({ code: "Backslash", key: "\\", ctrlKey: true });
    expect(readVibePreferences().panel["local:a"]).toBe("hidden");
    press({ code: "Backslash", key: "\\", ctrlKey: true });
    expect(readVibePreferences().panel["local:a"]).toBe("auto");
  });

  it("pins it for this conversation only", () => {
    mountWithPanel();
    press({ code: "Enter", key: "Enter" });
    press({ code: "Escape", key: "Escape" });
    press({ code: "Backslash", key: "\\", ctrlKey: true, shiftKey: true });
    expect(readVibePreferences().panel["local:a"]).toBe("pinned");
    // The other conversation is untouched: pinning is a fact about one conversation.
    expect(readVibePreferences().panel["local:b"]).toBeUndefined();
  });

  it("remembers what was chosen when the panel is opened again", () => {
    mountWithPanel();
    press({ code: "Enter", key: "Enter" });
    press({ code: "Escape", key: "Escape" });
    press({ code: "Backslash", key: "\\", ctrlKey: true, shiftKey: true });
    // Re-mount from storage, the way a relaunch does.
    mountWithPanel();
    press({ code: "Enter", key: "Enter" });
    press({ code: "Escape", key: "Escape" });
    // Pinned beats the narrow test window, which is the whole point of pinning.
    expect(readVibePreferences().panel["local:a"]).toBe("pinned");
  });

  it("offers the three states in the menu, adjustable with ← and →", () => {
    mountWithPanel();
    press({ code: "ContextMenu", key: "ContextMenu" });
    walkTo("panel");
    press({ code: "ArrowRight", key: "ArrowRight" });
    expect(readVibePreferences().panel["local:a"]).toBe("pinned");
  });
});

describe("the panel's top bar", () => {
  // macOS draws the traffic lights over the page's top-left (`titleBarStyle: "Overlay"`),
  // and the panel is the only surface whose content starts at x=0. Two things follow, and
  // both were broken: the content has to clear the lights, and the bar has to be the
  // window's drag handle (nothing else is left visible while the panel covers it).
  const bar = () => container.querySelector("header");

  it("starts past the traffic lights", () => {
    mount({ threads: [thread("a", 100)] });
    // This suite runs as non-macOS (`isMacOS` is false in the test environment), so the
    // ordinary inset is the expected one here; the macOS value is `barChrome.test.ts`.
    expect(bar()!.className).toContain("pl-3");
    expect(bar()!.className).not.toContain("pl-20");
  });

  it("is a drag handle", () => {
    // `startWindowDrag` skips buttons itself, so marking the whole bar is safe: the three
    // controls in it still receive their clicks.
    mount({ threads: [thread("a", 100)] });
    expect(bar()!.getAttribute("data-vibe-drag")).toBe("bar");
  });

  it("is as tall as every other top bar, so its text shares the lights' line", () => {
    // Reported from use: the text after the traffic lights looked misaligned. The bar was
    // `h-9` — text centred at 18px — while every other header in the app is `h-12` (24px),
    // which is the line the lights sit on. `barChrome.test.ts` holds the cross-file half;
    // this is the panel's own side of it.
    mount({ threads: [thread("a", 100)] });
    expect(bar()!.className).toContain("h-12");
    expect(bar()!.className).not.toContain("h-9");
  });
});

describe("where the grid puts its children", () => {
  // Reported from use as "the top row has no height". A child with only `gridColumn` keeps
  // an automatic row, so CSS placed the children with the auto-placement cursor — the add
  // tile took row one, the cells were pushed down, and the last of them landed in an
  // *implicit* row while the declared rows collapsed to 0. Measured on a real render:
  // `rows: 346px 346px 70px`. Every child now states both axes.
  const children = () => [...container.querySelectorAll<HTMLElement>(".grid > *")];

  it("states a row and a column for every cell and for the add tile", () => {
    // Three pinned conversations: a 2×2 with one free square, so the add tile is part of
    // the grid and is exactly the child that used to be placed first.
    mount({ threads: [thread("a", 100), thread("b", 200), thread("c", 300)] });
    const items = children();
    expect(items.length).toBeGreaterThan(1);
    for (const item of items) {
      expect(item.style.gridRow, `${item.tagName} needs an explicit row`).toBeTruthy();
      expect(item.style.gridColumn, `${item.tagName} needs an explicit column`).toBeTruthy();
    }
  });

  it("never asks for a row the template does not have", () => {
    // An implicit row is the failure mode, so the highest row any child claims must be one
    // the template declares (`rows`, from the layout the panel computed).
    mount({ threads: [thread("a", 100), thread("b", 200), thread("c", 300)] });
    const grid = container.querySelector<HTMLElement>(".grid")!;
    const rows = Number(grid.style.gridTemplateRows.match(/repeat\((\d+)/)![1]);
    const columns = Number(grid.style.gridTemplateColumns.match(/repeat\((\d+)/)![1]);
    for (const item of children()) {
      expect(Number(item.style.gridRow)).toBeLessThanOrEqual(rows);
      expect(Number(item.style.gridColumn)).toBeLessThanOrEqual(columns);
    }
  });

  it("gives no two children the same square", () => {
    mount({ threads: [thread("a", 100), thread("b", 200), thread("c", 300)] });
    const squares = children().map(item => `${item.style.gridRow}:${item.style.gridColumn}`);
    expect(new Set(squares).size).toBe(squares.length);
  });
});

describe("fullscreen from the bar", () => {
  // It did nothing for a release: the capability was missing, the rejection was swallowed,
  // and no test noticed. The three call sites are now one function that reports failure.
  it("reports a refused fullscreen instead of staying silent", async () => {
    // The capability layer rejecting `set_fullscreen` is exactly what happened in the
    // release, so that is the failure the test stages.
    h.win.setFullscreen.mockRejectedValueOnce(new Error("window.set_fullscreen not allowed"));
    const onToast = vi.fn();
    const off = onFutureEvent("toast", onToast);
    try {
      mount({ threads: [thread("a", 100)] });
      const button = container.querySelector<HTMLElement>("[aria-label=\"切换全屏\"], [aria-label=\"Toggle full screen\"]");
      expect(button, "the bar's fullscreen button").not.toBeNull();
      await act(async () => {
        button!.click();
        // The rejection is reported on a microtask; flush it.
        await Promise.resolve();
        await Promise.resolve();
      });
      expect(h.win.setFullscreen).toHaveBeenCalled();
      expect(onToast).toHaveBeenCalledWith(expect.objectContaining({ tone: "error" }));
    }
    finally {
      off();
    }
  });
});

describe("the second column's state carries over", () => {
  // Reported from use: closing the column in one cell then opening the next cell had it
  // open again, so it was closed twice. Hiding is a habit, not a fact about one
  // conversation — the *last* hidden/auto choice is the fallback for cells with no entry.
  it("keeps the column closed in the next conversation", () => {
    mount({ threads: [thread("a", 100), thread("b", 200)] });
    press({ code: "Enter", key: "Enter" });
    press({ code: "Escape", key: "Escape" });
    press({ code: "Backslash", key: "\\\\", ctrlKey: true });
    expect(readVibePreferences().panel["local:a"]).toBe("hidden");
    // Next conversation, never decided about: it follows the habit.
    press({ code: "ArrowRight", key: "ArrowRight" });
    press({ code: "Enter", key: "Enter" });
    press({ code: "Escape", key: "Escape" });
    expect(readVibePreferences().panelDefault).toBe("hidden");
    // …and the panel is showing it as closed, which is the thing a player would see.
    expect(container.querySelector("[data-vibe-panel-column]")).toBeNull();
  });

  it("opens again for the next conversation once the reader reopens it", () => {
    mount({ threads: [thread("a", 100), thread("b", 200)] });
    press({ code: "Enter", key: "Enter" });
    press({ code: "Escape", key: "Escape" });
    press({ code: "Backslash", key: "\\\\", ctrlKey: true });
    press({ code: "Backslash", key: "\\\\", ctrlKey: true });
    expect(readVibePreferences().panelDefault).toBe("auto");
  });

  it("does not inherit a pin", () => {
    // A pin is a decision about one conversation; inheriting it would open a column
    // nobody asked for in the next one.
    mount({ threads: [thread("a", 100), thread("b", 200)] });
    press({ code: "Enter", key: "Enter" });
    press({ code: "Escape", key: "Escape" });
    press({ code: "Backslash", key: "\\\\", ctrlKey: true, shiftKey: true });
    expect(readVibePreferences().panel["local:a"]).toBe("pinned");
    expect(readVibePreferences().panelDefault).toBe("auto");
  });
});

describe("reading a cell without zooming into it", () => {
  // The grid is the point of the panel: several conversations at once. Reading one of them
  // means moving *its* text, not the ring — so `PageUp`/`PageDown` scroll the focused cell.
  function transcript(): HTMLElement | null {
    return container.querySelector<HTMLElement>("[data-vibe-cell] [data-chat-scroll]");
  }

  it("scrolls the focused cell's transcript, not the ring", () => {
    mount({ threads: [thread("a", 100), thread("b", 200)] });
    const element = transcript();
    expect(element, "a transcript in the focused cell").not.toBeNull();
    // jsdom has no layout, so the call itself is the assertion: it targets this element,
    // by a page, and the ring does not move.
    const scrollBy = vi.fn();
    element!.scrollBy = scrollBy;
    // jsdom reports a zero-height box; the ratio is applied to whatever it reports, so a
    // fake height makes the expected delta explicit.
    Object.defineProperty(element!, "clientHeight", { configurable: true, value: 800 });

    press({ code: "PageDown", key: "PageDown" });
    expect(scrollBy).toHaveBeenCalledWith(expect.objectContaining({ top: Math.round(800 * 0.85) }));
    press({ code: "PageUp", key: "PageUp" });
    expect(scrollBy).toHaveBeenLastCalledWith(expect.objectContaining({ top: -Math.round(800 * 0.85) }));
    // The ring is still on the first cell: scrolling is not navigation.
    press({ code: "Enter", key: "Enter" });
    expect(drawn("input")).toEqual(["a"]);
  });

  it("scrolls with the arrows once the conversation fills the screen", () => {
    // In the reading layer ↑/↓ have nothing else to do (`←`/`→` switch conversations), so
    // they scroll — which is what a reader expects of a transcript they can see in full.
    mount({ threads: [thread("a", 100)] });
    press({ code: "Enter", key: "Enter" });
    press({ code: "Escape", key: "Escape" });
    const element = container.querySelector<HTMLElement>("[data-vibe-transcript-for=\"local:a\"] [data-chat-scroll]");
    expect(element).not.toBeNull();
    const scrollBy = vi.fn();
    element!.scrollBy = scrollBy;
    Object.defineProperty(element!, "clientHeight", { configurable: true, value: 600 });
    press({ code: "ArrowDown", key: "ArrowDown" });
    expect(scrollBy).toHaveBeenCalledWith(expect.objectContaining({ top: Math.round(600 * 0.85) }));
  });

  it("jumps to the newest message on the remote's home button", () => {
    mount({ threads: [thread("a", 100)] });
    const element = transcript();
    const scrollTo = vi.fn();
    element!.scrollTo = scrollTo;
    Object.defineProperty(element!, "scrollHeight", { configurable: true, value: 4000 });
    press({ code: "BrowserHome", key: "BrowserHome" });
    expect(scrollTo).toHaveBeenCalledWith(expect.objectContaining({ top: 4000 }));
  });
});

describe("the empty square", () => {
  // Reported from use: a 3×2 grid with five pinned conversations leaves one square empty,
  // and that square is where a reader looks for "add one". It is a *slot* in the ring, so
  // the arrows reach it and `⏎` opens the picker — not a floating button.
  const tile = () => container.querySelector("[data-vibe-add-slot]");

  it("fills a free square, and only when there is one", () => {
    // Five cells are 3×2: one square free.
    mount({ threads: [1, 2, 3, 4, 5].map(index => thread(`t${index}`, index * 100)) });
    expect(tile()).not.toBeNull();
    // Four cells are a full 2×2: no free square, so no tile (the menu's add row is the way
    // in). This is what keeps the layout table honest.
    mount({ threads: [1, 2, 3, 4].map(index => thread(`t${index}`, index * 100)) });
    expect(tile()).toBeNull();
  });

  /**
   * Put the ring on the tile by its printed number — the same rule a cell follows ("type
   * the number on the square"), and the only way that works for every shape: on a 2×2 the
   * tile is one `→` from the last cell, on a 3×2 it is `→` twice.
   */
  function focusTile(number: number) {
    press({ code: `Digit${number}`, key: String(number) });
    expect(container.querySelector("[data-vibe-focused]")?.getAttribute("data-vibe-focused")).toBe("__add__");
  }

  it("is reachable with the arrows and opens the picker on ⏎", () => {
    mount({ threads: [1, 2, 3].map(index => thread(`t${index}`, index * 100)) });
    // Three cells are 2×2, so the tile is the fourth slot.
    focusTile(4);
    press({ code: "Enter", key: "Enter" });
    expect(container.querySelector("[role=\"menu\"]")).not.toBeNull();
    // The picker always offers the two ways out, even with nothing recent to offer.
    expect(container.querySelector("[data-vibe-menu-row=\"add:browse\"]")).not.toBeNull();
    expect(container.querySelector("[data-vibe-menu-row=\"add:new\"]")).not.toBeNull();
  });

  it("lists recently active conversations, and pins the one chosen", () => {
    const onPinLocal = vi.fn();
    mount({
      onPinLocal,
      threads: [
        thread("a", 100),
        thread("b", 200),
        thread("c", 300),
        // Unpinned, so candidates: the most recently touched wins the first row.
        thread("older", 50, { lastMessageAt: 300, pinned: false }),
        thread("newer", 60, { lastMessageAt: 900, pinned: false }),
      ],
    });
    // Three pinned cells are a 2×2 with one square free: the tile is the fourth slot. The
    // two candidates below are unpinned, which is what makes them candidates at all.
    focusTile(4);
    press({ code: "Enter", key: "Enter" });
    const rows = [...container.querySelectorAll("[data-vibe-menu-row]")].map(row => row.getAttribute("data-vibe-menu-row"));
    expect(rows[0]).toBe("recent:local:newer");
    expect(rows[1]).toBe("recent:local:older");
    press({ code: "Enter", key: "Enter" });
    expect(onPinLocal).toHaveBeenCalledWith("newer", true);
  });

  it("offers a way to the full picker and to a new conversation", () => {
    const onNewConversation = vi.fn();
    mount({ onNewConversation, threads: [thread("a", 100)] });
    // One cell is a full 1×1, so reach the picker the way a reader would: the panel menu.
    press({ code: "ContextMenu", key: "ContextMenu" });
    walkTo("add");
    press({ code: "Enter", key: "Enter" });
    // The browse dialog is the shell's; the new-conversation hand-off is the panel's.
    expect(container.querySelector("[role=\"dialog\"]")).not.toBeNull();
    press({ code: "Escape", key: "Escape" });
    mount({ onNewConversation, threads: [thread("a", 100), thread("b", 200)] });
    press({ code: "ContextMenu", key: "ContextMenu" });
    walkTo("add");
    press({ code: "Enter", key: "Enter" });
    press({ code: "Escape", key: "Escape" });
  });
});

describe("more than a page of pinned conversations", () => {
  // Twelve cells was a silent cap: a thirteenth pinned conversation was unreachable and the
  // bar said "1 more than the grid holds". The grid now pages.
  const thirteen = () => Array.from({ length: 13 }, (_, index) => thread(`t${String(index).padStart(2, "0")}`, (index + 1) * 100));

  it("shows twelve, says which page, and reaches the rest", () => {
    mount({ threads: thirteen() });
    expect(container.querySelectorAll("[data-vibe-cell]")).toHaveLength(12);
    expect(container.querySelector("[data-vibe-page]")?.textContent).toContain("1/2");
    press({ code: "ArrowRight", key: "ArrowRight", altKey: true });
    expect(container.querySelectorAll("[data-vibe-cell]")).toHaveLength(1);
    expect(container.querySelector("[data-vibe-page]")?.textContent).toContain("2/2");
    // The thirteenth is the one on page two, and it is reachable.
    press({ code: "Enter", key: "Enter" });
    expect(drawn("input")).toEqual(["t12"]);
  });

  it("falls back a page when the reader unpins their way back to twelve", () => {
    // An unpin on page two that leaves exactly one page must not strand the reader on an
    // empty grid.
    mount({ threads: thirteen() });
    press({ code: "ArrowRight", key: "ArrowRight", altKey: true });
    expect(container.querySelector("[data-vibe-page]")?.textContent).toContain("2/2");
    mount({ threads: thirteen().slice(0, 12) });
    expect(container.querySelectorAll("[data-vibe-cell]")).toHaveLength(12);
    expect(container.querySelector("[data-vibe-page]")).toBeNull();
  });

  it("does not offer the page keys with a single page", () => {
    // The panel's rule for every command: a key that could not do anything stays the app's.
    mount({ threads: [thread("a", 100)] });
    expect(container.querySelector("[data-vibe-page]")).toBeNull();
    press({ code: "ArrowRight", key: "ArrowRight", altKey: true });
    expect(drawn("read")).toEqual(["a"]);
  });
});

describe("membership", () => {
  it("shows only pinned, live conversations of this machine", () => {
    mount({
      threads: [
        thread("b", 200),
        thread("a", 100),
        thread("unpinned", 50, { pinned: false }),
        thread("archived", 60, { pinned: true, status: "archived" }),
      ],
    });
    expect(drawn("read")).toEqual(["a", "b"]);
  });

  it("says so when nothing is pinned, and draws no cell", () => {
    mount({ threads: [thread("unpinned", 50, { pinned: false })] });
    expect(chromes()).toEqual([]);
    expect(container.textContent).toContain("No pinned conversations");
  });

  it("unpins the focused conversation on the remove key", () => {
    const onPinLocal = vi.fn();
    mount({ onPinLocal, threads: [thread("a", 100), thread("b", 200)] });
    press({ code: "Digit2", key: "2" });
    press({ code: "Backspace", key: "Backspace", ctrlKey: true, shiftKey: true });
    expect(onPinLocal).toHaveBeenCalledWith("b", false);
  });
});

describe("order", () => {
  it("defaults to creation time, oldest first", () => {
    mount({ threads: [thread("c", 300), thread("a", 100), thread("b", 200)] });
    expect(drawn("read")).toEqual(["a", "b", "c"]);
  });

  it("moves the selected cell and stores the order", () => {
    mount({ threads: [thread("a", 100), thread("b", 200), thread("c", 300)] });
    press({ code: "Digit1", key: "1" });
    // mod+alt+→ moves the selected cell later; the panel's scope owns that key.
    press({ code: "ArrowRight", key: "ArrowRight", ctrlKey: true, altKey: true });
    expect(drawn("read")).toEqual(["b", "a", "c"]);
    expect(readVibePreferences().order).toEqual(["local:b", "local:a", "local:c"]);
  });

  it("puts a newly pinned conversation where it belongs in time", () => {
    mount({ threads: [thread("a", 100), thread("c", 300)] });
    // The user moves c before a …
    press({ code: "Digit2", key: "2" });
    press({ code: "ArrowLeft", key: "ArrowLeft", ctrlKey: true, altKey: true });
    expect(drawn("read")).toEqual(["c", "a"]);
    // … then pins one that is older than both. It joins by its creation time
    // instead of being appended.
    mount({ threads: [thread("a", 100), thread("b", 50), thread("c", 300)] });
    expect(drawn("read")).toEqual(["b", "c", "a"]);
  });

  it("drops a conversation that is unpinned elsewhere without moving the rest", () => {
    mount({ threads: [thread("a", 100), thread("b", 200), thread("c", 300)] });
    press({ code: "Digit3", key: "3" });
    press({ code: "ArrowLeft", key: "ArrowLeft", ctrlKey: true, altKey: true });
    expect(drawn("read")).toEqual(["a", "c", "b"]);
    mount({ threads: [thread("b", 200), thread("c", 300)] });
    expect(drawn("read")).toEqual(["c", "b"]);
  });

  it("keeps the ring on the same conversation when the order changes", () => {
    // The ring is held by key, not by index: a reorder must not silently select a
    // different conversation.
    mount({ threads: [thread("a", 100), thread("b", 200), thread("c", 300)] });
    press({ code: "Digit2", key: "2" });
    press({ code: "ArrowLeft", key: "ArrowLeft", ctrlKey: true, altKey: true });
    press({ code: "Enter", key: "Enter" });
    expect(drawn("input")).toEqual(["b"]);
  });
});

describe("the panel's own commands with nothing under them", () => {
  // Every one of these is a guard, not a feature: the panel registers a command
  // whenever its *layer* is open, and the thing the command acts on can be gone by the
  // time a key reaches it (nothing pinned, no ring, a page that does not exist). What
  // they must never do is throw or write a preference for a cell that is not there.
  const emptyPrefs = { columns: null, device: { kind: "all" }, order: [], panel: {}, panelDefault: "auto" };

  it("keeps the ring, the order and the column keys quiet on a panel with no cells", () => {
    mount({ threads: [] });
    // `moveFocus`: there is no cell to move to.
    act(() => void runCommand("vibe.focus.left"));
    // `setPanelPreference`: no focused conversation, so there is no choice to remember.
    act(() => void runCommand("vibe.panel.hide"));
    act(() => void runCommand("vibe.panel.pin"));
    // `vibe.move.left/right`: nothing to reorder.
    act(() => void runCommand("vibe.move.left"));
    act(() => void runCommand("vibe.move.right"));

    expect(readVibePreferences()).toEqual(emptyPrefs);
  });

  it("stays put when the ring is already at the edge", () => {
    // One cell, so every direction is the same square: the key must not clear the ring
    // (which would drop the reader out of the cell they are looking at).
    mount({ threads: [thread("a", 100)] });
    act(() => void runCommand("vibe.focus.left"));
    act(() => void runCommand("vibe.focus.up"));
    press({ code: "Enter", key: "Enter" });
    expect(drawn("input")).toEqual(["a"]);
  });

  it("does not step past the ends of the row while reading", () => {
    mount({ threads: [thread("a", 100)] });
    press({ code: "Enter", key: "Enter" });
    press({ code: "Escape", key: "Escape" });
    // `stepFocus` clamps, and a single cell means both directions land where it already is.
    act(() => void runCommand("vibe.zoom.previous"));
    act(() => void runCommand("vibe.zoom.next"));
    expect(drawn("read")).toEqual(["a"]);
  });

  it("steps through nothing when the last cell is unpinned from the reading layer", () => {
    // The layer is a state of the panel, not of the cells: unpinning the conversation
    // being read leaves the reading layer open with no cells in it.
    mount({ threads: [thread("a", 100)] });
    press({ code: "Enter", key: "Enter" });
    press({ code: "Escape", key: "Escape" });
    rerender({ threads: [] });

    act(() => void runCommand("vibe.zoom.next"));

    expect(drawn("read")).toEqual([]);
  });

  it("ignores a digit that is not a cell number", () => {
    mount({ threads: [thread("a", 100), thread("b", 200)] });
    const binding = (code: string) => ({ alt: false, code, ctrl: false, mod: false, shift: false });
    // `0` is the key left of `1`, and a letter carries no number at all.
    act(() => void runCommand("vibe.cell", binding("Digit0")));
    act(() => void runCommand("vibe.cell", binding("KeyA")));
    // The ring stayed on the first cell.
    press({ code: "Enter", key: "Enter" });
    expect(drawn("input")).toEqual(["a"]);
  });

  it("falls back to the first cell when the tile the ring sits on has been filled", () => {
    // The ring is on the add tile of a 2×2 with one free square; pinning a fourth
    // conversation fills that square, so the tile — and the key the ring was on — is gone.
    // The ring lands on the first cell rather than on nothing: a ring on "no cell" would
    // leave `⏎` with nowhere to go on a panel the reader can still see.
    mount({ threads: [thread("a", 100), thread("b", 200), thread("c", 300)] });
    act(() => void runCommand("vibe.cell", { alt: false, code: "Digit4", ctrl: false, mod: false, shift: false }));

    rerender({ threads: [thread("a", 100), thread("b", 200), thread("c", 300), thread("d", 400)] });
    act(() => void runCommand("vibe.open"));

    expect(drawn("input")).toEqual(["a"]);
  });

  it("does not turn past the last page", () => {
    // Thirteen conversations are two pages; the second page's `next` is the same page.
    const many = Array.from({ length: 13 }, (_, index) => thread(`t${index}`, 100 + index));
    mount({ threads: many });
    act(() => void runCommand("vibe.page.next"));
    // Thirteen conversations are twelve on the first page and one on the second.
    const secondPage = drawn("read");
    expect(secondPage).toEqual(["t12"]);

    act(() => void runCommand("vibe.page.next"));

    // Still the second page, and still ringing the same conversation: `next` at the end
    // is the page it is already on.
    expect(drawn("read")).toEqual(secondPage);
  });
});

describe("the grid area it measures", () => {
  it("lays the cells out from the area's own size, and stops measuring when unmounted", () => {
    // The grid area is smaller than the window (the bar above it and the padding around
    // it are what a cell does not get), which is the whole reason it is measured. jsdom
    // has no ResizeObserver, so this is also the branch that never runs in the app's
    // tests — and the one a missing registration would leave silently stale.
    const observed: Array<() => void> = [];
    const disconnect = vi.fn();
    const observe = vi.fn();
    class FakeResizeObserver {
      constructor(callback: () => void) {
        observed.push(callback);
      }

      disconnect = disconnect;
      observe = observe;
      unobserve = vi.fn();
    }
    vi.stubGlobal("ResizeObserver", FakeResizeObserver);
    try {
      mount({ threads: [thread("a", 100), thread("b", 200), thread("c", 300), thread("d", 400)] });
      expect(observe).toHaveBeenCalledTimes(1);
      const area = observe.mock.calls[0]![0] as HTMLElement;
      // The element it watches is the one the cells are drawn in, not the window.
      expect(area.contains(container.querySelector("[data-chrome]"))).toBe(true);

      Object.defineProperty(area, "clientHeight", { configurable: true, value: 300 });
      Object.defineProperty(area, "clientWidth", { configurable: true, value: 400 });
      act(() => observed.forEach(callback => callback()));

      // The shape now follows the measured area rather than the window's 1024×768.
      const grid = container.querySelector<HTMLElement>(".grid")!;
      const expected = gridOf(4, 400, 300, null);
      expect(grid.style.gridTemplateColumns).toBe(`repeat(${expected.columns}, minmax(0, 1fr))`);
    }
    finally {
      act(() => root.unmount());
      container.remove();
      uninstallShortcuts();
      vi.unstubAllGlobals();
    }
    expect(disconnect).toHaveBeenCalled();

    // `afterEach` unmounts again; keep it harmless by re-creating what it removes.
    container = document.body.appendChild(document.createElement("div"));
    root = createRoot(container);
    uninstallShortcuts = installShortcuts();
  });
});

describe("the menu's rows, run rather than adjusted", () => {
  // Each row has two faces: `←`/`→` adjusts the value in place, `⏎` runs the row's own
  // action. The adjust half is covered above; this is the run half, which is what a
  // pointer and a remote's OK button actually press.
  function openMenu() {
    press({ code: "ContextMenu", key: "ContextMenu" });
  }

  it("returns the layout to automatic from its row", () => {
    mount({ threads: [thread("a", 100), thread("b", 200), thread("c", 300)] });
    openMenu();
    walkTo("layout");
    // A manual column count first, so "automatic" is a change rather than the state it
    // was already in.
    press({ code: "ArrowRight", key: "ArrowRight" });
    expect(readVibePreferences().columns).toBe(1);

    press({ code: "Enter", key: "Enter" });

    expect(readVibePreferences().columns).toBeNull();
    // The sheet stays up: a layout change is something the reader watches happen behind it.
    expect(container.querySelector("[role=\"menu\"]")).not.toBeNull();
  });

  it("goes back to every machine from the device row", () => {
    mount({
      catalogs: [{ desktopId: "desk_1", sessions: [] } as never],
      peers: [{ desktopId: "desk_1", name: "Studio" } as never],
      threads: [thread("a", 100)],
    });
    openMenu();
    walkTo("device");
    press({ code: "ArrowRight", key: "ArrowRight" });
    expect(readVibePreferences().device).toEqual({ desktopId: "local", kind: "device" });

    press({ code: "Enter", key: "Enter" });

    expect(readVibePreferences().device).toEqual({ kind: "all" });
  });

  it("hides the second column from its row, and remembers the sticky default", () => {
    mount({ threads: [thread("a", 100), thread("b", 200)] });
    openMenu();
    walkTo("panel");
    press({ code: "Enter", key: "Enter" });

    // One write for the conversation's choice and the default the next cell follows.
    expect(readVibePreferences()).toEqual(expect.objectContaining({
      panel: { "local:a": "hidden" },
      panelDefault: "hidden",
    }));
  });

  it("moves the focused cell later in the panel from its row", () => {
    mount({ threads: [thread("a", 100), thread("b", 200), thread("c", 300)] });
    openMenu();
    walkTo("move-later");
    press({ code: "Enter", key: "Enter" });

    // The focused cell (the first) moves one place later, and the rest keep their order.
    expect(readVibePreferences().order).toEqual(["local:b", "local:a", "local:c"]);
  });

  it("opens the focused conversation in the shell from its row", () => {
    const onOpenInShell = vi.fn();
    mount({ onOpenInShell, threads: [thread("a", 100)] });
    openMenu();
    walkTo("open-in-shell");
    press({ code: "Enter", key: "Enter" });

    expect(onOpenInShell).toHaveBeenCalledWith(expect.objectContaining({ id: "a" }));
  });

  it("opens a host's conversation in the shell on that host", () => {
    const onOpenRemoteInShell = vi.fn();
    mount({
      catalogs: [{ desktopId: "desk_1", sessions: [{ pinned: true, sessionId: "s1", status: "completed", title: "Remote" }] } as never],
      onOpenRemoteInShell,
      peers: [{ desktopId: "desk_1", name: "Studio" } as never],
      threads: [],
    });
    openMenu();
    walkTo("open-in-shell");
    press({ code: "Enter", key: "Enter" });

    expect(onOpenRemoteInShell).toHaveBeenCalledWith("desk_1", "s1");
  });
});

describe("the add tile's menu", () => {
  // The tile's menu is the keyboard's way into the picker: a remote has four arrows and
  // OK, and a dialog with three columns is not something they can drive.
  /** Three cells make a 2×2 with one free square, so the tile exists: `4` rings it. */
  function openTileMenu() {
    press({ code: "Digit4", key: "4" });
    press({ code: "Enter", key: "Enter" });
  }

  it("starts a new conversation from its own row", () => {
    const onNewConversation = vi.fn();
    mount({ onNewConversation, threads: [thread("a", 100), thread("b", 200), thread("c", 300)] });
    openTileMenu();
    walkTo("add:new");
    press({ code: "Enter", key: "Enter" });

    expect(onNewConversation).toHaveBeenCalledTimes(1);
    // Both sheets are gone: the reader asked to leave, not to keep reading the list.
    expect(container.querySelector("[role=\"menu\"]")).toBeNull();
  });

  it("opens the full picker from the browse row", () => {
    mount({ threads: [thread("a", 100), thread("b", 200), thread("c", 300)] });
    openTileMenu();
    walkTo("add:browse");
    press({ code: "Enter", key: "Enter" });

    expect(container.querySelector("[role=\"menu\"]")).toBeNull();
    // The dialog, not the tile's short list: it can add a conversation from any machine.
    expect(container.querySelector("[data-add-machine]")).not.toBeNull();
  });

  it("offers a machine's workspaces and filters its conversations by hand", () => {
    // The dialog is the pointer's way in, and it is three columns: this machine (or a
    // host), the workspaces of that machine, then the conversations. A host's
    // workspaces come from the catalogue it already sent — this app has no other list.
    mount({
      catalogs: [{
        desktopId: "desk_1",
        sessions: [
          { pinned: false, sessionId: "s1", status: "completed", title: "In a workspace", workspaceId: "ws_remote", workspaceName: "future-os" },
          { pinned: false, sessionId: "s2", status: "completed", title: "Other two", workspaceId: "ws_remote", workspaceName: "future-os" },
          { pinned: false, sessionId: "s3", status: "completed", title: "No workspace" },
        ],
      } as never],
      peers: [{ desktopId: "desk_1", name: "Studio" } as never],
      threads: [thread("a", 100), thread("b", 200)],
      workspaces: [
        { id: "ws", kind: "user", name: "future-os", path: "/tmp/future-os" } as unknown as StoredWorkspace,
        { id: "ws_chat", kind: "chat", name: "Chats", path: "/tmp/chats" } as unknown as StoredWorkspace,
      ],
    });
    press({ code: "ContextMenu", key: "ContextMenu" });
    walkTo("add");
    press({ code: "Enter", key: "Enter" });

    // This machine's column: chats, then the user's workspaces (a chat file is not a
    // workspace the reader can pick).
    expect(container.querySelector("[data-add-workspace=\"ws\"]")).not.toBeNull();
    expect(container.querySelector("[data-add-workspace=\"ws_chat\"]")).toBeNull();

    // A host's column: the same shape, named from its own catalogue — and two
    // conversations in one workspace are one row, not two.
    act(() => container.querySelector<HTMLElement>("[data-add-machine=\"desk_1\"]")!.click());
    const remoteWorkspaces = [...container.querySelectorAll("[data-add-workspace]")]
      .map(node => node.getAttribute("data-add-workspace"));
    expect(remoteWorkspaces.filter(id => id === "ws_remote")).toHaveLength(1);
    expect(container.querySelector("[data-add-workspace=\"ws\"]")).toBeNull();

    act(() => container.querySelector<HTMLElement>("[data-add-workspace=\"ws_remote\"]")!.click());
    expect(container.textContent).toContain("In a workspace");
    expect(container.textContent).not.toContain("No workspace");

    // And the filter narrows what is left, by title.
    const filter = container.querySelector<HTMLInputElement>("input")!;
    const setValue = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!;
    act(() => {
      setValue.call(filter, "other");
      filter.dispatchEvent(new Event("input", { bubbles: true }));
    });
    expect(container.textContent).not.toContain("In a workspace");
  });

  it("adds a recent conversation from the tile's list, and rings it", () => {
    const onPinLocal = vi.fn();
    mount({
      onPinLocal,
      threads: [
        thread("a", 100),
        thread("b", 200),
        thread("c", 300),
        thread("candidate", 400, { pinned: false, title: "Candidate" }),
      ],
    });
    openTileMenu();
    walkTo("recent:local:candidate");
    press({ code: "Enter", key: "Enter" });

    expect(onPinLocal).toHaveBeenCalledWith("candidate", true);
    expect(container.querySelector("[role=\"menu\"]")).toBeNull();
  });

  it("adds a host's recent conversation on that host", () => {
    const onPinRemote = vi.fn();
    mount({
      catalogs: [{
        desktopId: "desk_1",
        sessions: [
          { pinned: true, sessionId: "s0", status: "completed", title: "Remote, pinned" },
          { pinned: false, sessionId: "s1", status: "completed", title: "Remote candidate" },
        ],
      } as never],
      onPinRemote,
      peers: [{ desktopId: "desk_1", name: "Studio" } as never],
      threads: [thread("a", 100), thread("b", 200)],
    });
    openTileMenu();
    walkTo("recent:remote:desk_1:s1");
    press({ code: "Enter", key: "Enter" });

    expect(onPinRemote).toHaveBeenCalledWith("desk_1", "s1", true);
  });
});

describe("the cell's two writers, from the menu's rings", () => {
  const MODEL_OPTIONS = [
    { id: "deepseek-v4-pro", label: "DeepSeek V4 Pro", provider: "future" },
    { id: "claude-sonnet", label: "Claude Sonnet", provider: "future" },
  ] as never;

  function mountWithModels() {
    h.cached.current = { model: "future/deepseek-v4-pro", thinkingLevel: "medium" };
    mount({ threads: [thread("a", 100)], view: { modelOptions: MODEL_OPTIONS } });
  }

  it("hands the ring's next model to that conversation's writer", () => {
    mountWithModels();
    press({ code: "ContextMenu", key: "ContextMenu" });
    walkTo("model");
    press({ code: "ArrowRight", key: "ArrowRight" });

    // The conversation's own id — not the shell's "active thread" — and the catalogue it
    // was chosen from, so the level can be re-derived for the new model.
    expect(h.setters.model).toHaveBeenLastCalledWith("a", "future/claude-sonnet", MODEL_OPTIONS);
  });

  it("wraps the model ring in both directions", () => {
    mountWithModels();
    press({ code: "ContextMenu", key: "ContextMenu" });
    walkTo("model");
    // Two models: one step left from the first is the last.
    press({ code: "ArrowLeft", key: "ArrowLeft" });

    expect(h.setters.model).toHaveBeenLastCalledWith("a", "future/claude-sonnet", MODEL_OPTIONS);
  });

  it("hands the ring's next thinking level to that conversation's writer", () => {
    mountWithModels();
    press({ code: "ContextMenu", key: "ContextMenu" });
    walkTo("thinking");
    press({ code: "ArrowRight", key: "ArrowRight" });

    expect(h.setters.thinking).toHaveBeenCalledWith("a", expect.any(String));
    // Not the level it was already on: the ring moved.
    expect(h.setters.thinking.mock.calls[0]![1]).not.toBe("medium");
  });
});

describe("what a cell reports back to the panel", () => {
  it("answers an approval through the panel's queue and tells the conversation", async () => {
    const onApprovalDecided = vi.fn();
    const approval = { id: "ap_1" };
    h.approvals.activeApproval = approval;
    mount({ threads: [thread("a", 100)], view: { onApprovalDecided } });
    // The reading layer, because the grid hands every cell `null` (one queue, not twelve).
    press({ code: "Enter", key: "Enter" });
    press({ code: "Escape", key: "Escape" });

    const probe = container.querySelector<HTMLElement>("[data-approval-probe]");
    expect(probe, "a cell that can answer").not.toBeNull();
    await act(async () => {
      probe!.click();
    });

    expect(h.approvals.decideApproval).toHaveBeenCalledWith(approval, "approved");
    // The conversation is told after the decision lands, not before: it is what closes
    // the card there.
    expect(onApprovalDecided).toHaveBeenCalledTimes(1);
    h.approvals.activeApproval = null;
  });

  it("leaves the input layer when the editor says it was left", () => {
    // An empty draft closed with `⏎` reports through the composer's own callback rather
    // than through a command, so the panel has to honour it.
    mount({ threads: [thread("a", 100)] });
    press({ code: "Enter", key: "Enter" });
    expect(drawn("input")).toEqual(["a"]);

    const probe = container.querySelector<HTMLElement>("[data-leave-input-probe]");
    expect(probe, "an editor that reports being left").not.toBeNull();
    act(() => probe!.click());

    expect(drawn("input")).toEqual([]);
    expect(drawn("read")).toEqual(["a"]);
  });
});

describe("the column commands", () => {
  // Five ways to say how many columns: cycle (1→2→3→4→automatic), more, fewer, and back
  // to automatic. Each writes immediately, so a relaunch lands on the last one pressed.
  const columnsPref = () => readVibePreferences().columns;

  it("cycles 1 → 2 → 3 → 4 → automatic", () => {
    mount({ threads: [thread("a", 100)] });
    const seen: Array<number | null> = [];
    for (let step = 0; step < 5; step++) {
      act(() => void runCommand("vibe.columns.cycle"));
      seen.push(columnsPref());
    }
    // The first press moves off the automatic layout, and the fifth comes back to it.
    expect(seen).toEqual([2, 3, 4, null, 2]);
  });

  it("adds a column, up to four, and takes one away, down to one", () => {
    mount({ threads: [thread("a", 100)] });
    for (let step = 0; step < 6; step++)
      act(() => void runCommand("vibe.columns.more"));
    // Four is the cap: more columns than that are not readable at this size.
    expect(columnsPref()).toBe(4);

    for (let step = 0; step < 5; step++)
      act(() => void runCommand("vibe.columns.fewer"));
    expect(columnsPref()).toBe(1);
  });

  it("goes back to the automatic layout from a manual count", () => {
    mount({ threads: [thread("a", 100)] });
    act(() => void runCommand("vibe.columns.more"));
    expect(columnsPref()).toBeGreaterThan(0);

    act(() => void runCommand("vibe.columns.auto"));

    expect(columnsPref()).toBeNull();
  });

  it("counts from the layout the panel computed when no count was chosen", () => {
    // Automatic is `null`, so "more" has to start from what the grid is drawing rather
    // than from zero: 0 + 1 would be a *fewer* layout.
    mount({ threads: [thread("a", 100)] });
    act(() => void runCommand("vibe.columns.more"));
    const automatic = gridOf(1, window.innerWidth, window.innerHeight, null);
    expect(columnsPref()).toBe(Math.min(4, automatic.columns + 1));
  });
});

describe("the pointer's ways in", () => {
  it("rings the cell that was pressed and opens the reading layer on it", () => {
    mount({ threads: [thread("a", 100), thread("b", 200)] });
    const cells = () => [...container.querySelectorAll<HTMLElement>("[data-vibe-cell]")];
    act(() => {
      cells()[1]!.dispatchEvent(new MouseEvent("mousedown", { bubbles: true }));
    });

    // The press both rings the cell and opens it: a pointer has no separate `⏎`.
    expect(drawn("read")).toEqual(["b"]);
    expect(container.querySelector("[data-vibe-layer]")?.getAttribute("data-vibe-layer")).toBe("read");
  });

  it("opens the tile's menu from the tile itself", () => {
    mount({ threads: [thread("a", 100), thread("b", 200), thread("c", 300)] });
    const tile = container.querySelector<HTMLElement>("[data-vibe-add-slot]")!;
    expect(tile).not.toBeNull();

    act(() => {
      tile.dispatchEvent(new MouseEvent("mousedown", { bubbles: true }));
      tile.click();
    });

    // The tile's own menu, not the panel's: the rows are the ones it can add.
    expect(container.querySelector("[data-vibe-menu-row=\"add:new\"]")).not.toBeNull();
  });

  it("opens a conversation in the shell by double-clicking its header", () => {
    const onOpenInShell = vi.fn();
    const onOpenRemoteInShell = vi.fn();
    mount({
      catalogs: [{ desktopId: "desk_1", sessions: [{ pinned: true, sessionId: "s1", status: "completed", title: "Remote" }] } as never],
      onOpenInShell,
      onOpenRemoteInShell,
      peers: [{ desktopId: "desk_1", name: "Studio" } as never],
      threads: [thread("a", 100)],
    });
    const headers = () => [...container.querySelectorAll<HTMLElement>("[data-vibe-cell] header")];
    act(() => {
      headers()[0]!.dispatchEvent(new MouseEvent("dblclick", { bubbles: true }));
    });
    act(() => {
      headers()[1]!.dispatchEvent(new MouseEvent("dblclick", { bubbles: true }));
    });

    expect(onOpenInShell).toHaveBeenCalledWith(expect.objectContaining({ id: "a" }));
    // A host's conversation opens on the host, as the host's session.
    expect(onOpenRemoteInShell).toHaveBeenCalledWith("desk_1", "s1");
  });

  it("opens the menu and the picker from the bar and the empty state", () => {
    mount({ threads: [thread("a", 100)] });
    const barButton = (label: string) => [...container.querySelectorAll<HTMLElement>("header button")]
      .find(button => button.getAttribute("aria-label") === label)!;

    act(() => barButton("Panel menu").click());
    expect(container.querySelector("[role=\"menu\"]")).not.toBeNull();
    act(() => barButton("Panel menu").click());

    // The empty panel offers the picker as its only way forward.
    rerender({ threads: [] });
    const add = [...container.querySelectorAll<HTMLElement>("button")]
      .find(button => button.textContent === "Add a conversation")!;
    act(() => add.click());
    expect(container.querySelector("[data-add-machine]")).not.toBeNull();
  });

  it("opens the focused conversation in the shell by double-clicking the transcript", () => {
    const onOpenInShell = vi.fn();
    const onOpenRemoteInShell = vi.fn();
    mount({ onOpenInShell, onOpenRemoteInShell, threads: [thread("a", 100)] });
    press({ code: "Enter", key: "Enter" });
    press({ code: "Escape", key: "Escape" });
    const transcript = container.querySelector<HTMLElement>("[data-vibe-transcript-for]")!;

    act(() => {
      transcript.dispatchEvent(new MouseEvent("dblclick", { bubbles: true }));
    });

    expect(onOpenInShell).toHaveBeenCalledWith(expect.objectContaining({ id: "a" }));
  });
});

describe("the reading layer's second column when it is pinned", () => {
  const column = () => container.querySelector("[data-vibe-column-stub]");

  it("draws this machine's own panel for a local conversation", () => {
    // jsdom's window is narrower than the automatic rule needs, so the pin is what puts
    // the column on screen — and a pin is a per-conversation choice. The conversation
    // belongs to a workspace, so the panel is handed the workspace itself rather than
    // "none": a files column with no root is a column with nothing in it.
    mount({
      threads: [thread("a", 100)],
      workspaces: [{ id: "ws", name: "future-os", path: "/tmp/future-os" } as StoredWorkspace],
    });
    press({ code: "Enter", key: "Enter" });
    press({ code: "Escape", key: "Escape" });
    act(() => void runCommand("vibe.panel.pin"));

    expect(column()).not.toBeNull();
  });

  it("closes the column from the panel's own toggle", () => {
    mount({ threads: [thread("a", 100)] });
    press({ code: "Enter", key: "Enter" });
    press({ code: "Escape", key: "Escape" });
    act(() => void runCommand("vibe.panel.pin"));
    const toggle = column()!.querySelector<HTMLElement>("[data-toggle-expanded]");

    expect(toggle, "the column's collapse control").not.toBeNull();
    act(() => toggle!.click());

    // The sticky default follows the toggle, so the next conversation opens closed too.
    expect(readVibePreferences()).toEqual(expect.objectContaining({
      panel: { "local:a": "hidden" },
      panelDefault: "hidden",
    }));
  });

  it("draws the host's file panel for a host's conversation", () => {
    mount({
      catalogs: [{ desktopId: "desk_1", sessions: [{ pinned: true, sessionId: "s1", status: "completed", title: "Remote" }] } as never],
      peers: [{ desktopId: "desk_1", name: "Studio" } as never],
      threads: [],
    });
    // A host's conversation has no composer here, so `⏎` opens it for reading directly.
    press({ code: "Enter", key: "Enter" });
    act(() => void runCommand("vibe.panel.pin"));

    // `RemoteFilesPanel` is stubbed the same way: what matters is that the host's
    // conversation gets the host's panel rather than this machine's files.
    expect(container.querySelectorAll("[data-vibe-column-stub]")).toHaveLength(1);

    // And it closes by the same sticky rule as this machine's panel.
    act(() => container.querySelector<HTMLElement>("[data-toggle-expanded]")!.click());
    expect(readVibePreferences()).toEqual(expect.objectContaining({
      panel: { "remote:desk_1:s1": "hidden" },
      panelDefault: "hidden",
    }));
  });
});

describe("two more of the menu's rows", () => {
  it("moves a later cell earlier from its row", () => {
    mount({ threads: [thread("a", 100), thread("b", 200), thread("c", 300)] });
    // Ring the second cell, so "move earlier" is not the row that cannot run.
    act(() => void runCommand("vibe.focus.right"));
    press({ code: "ContextMenu", key: "ContextMenu" });
    walkTo("move-earlier");
    press({ code: "Enter", key: "Enter" });

    expect(readVibePreferences().order).toEqual(["local:b", "local:a", "local:c"]);
  });

  it("leaves the model row inert while the catalogue is still empty", () => {
    // A shell that has not loaded its models yet: the row is drawn (the conversation is
    // this machine's) but there is nothing to cycle to, so the ring must not write an id.
    h.cached.current = { model: "future/unknown", thinkingLevel: "medium" };
    mount({ threads: [thread("a", 100)] });
    press({ code: "ContextMenu", key: "ContextMenu" });
    walkTo("model");
    press({ code: "ArrowRight", key: "ArrowRight" });

    expect(h.setters.model).not.toHaveBeenCalled();
  });
});

describe("a held sentence the reader has started writing over", () => {
  it("keeps it queued and says why, instead of overwriting the box", () => {
    // The refusal is the queue's `busy`: the box holds something typed since, and
    // putting the held sentence back would replace it.
    const toasts: Array<{ message: string }> = [];
    const dispose = onFutureEvent("toast", (payload) => {
      toasts.push(payload as { message: string });
    });
    try {
      mount({ runStatuses: { a: { status: "running" } }, threads: [thread("a", 100)] });
      // The sentence is handed over from the grid's composer, then the reader goes back
      // into the editor and starts typing something of their own.
      act(() => container.querySelector<HTMLElement>("[data-queue-probe]")!.click());
      press({ code: "Enter", key: "Enter" });
      saveComposerDraft("a", { attachments: [], text: "something I am writing" });

      // The cancel key, as a press rather than as a direct call: the row is only
      // registered while a sentence is held.
      press({ code: "KeyX", key: "x", ctrlKey: !isMacOS, metaKey: isMacOS, shiftKey: true });

      // The refusal is spoken, and the box keeps what the reader was writing.
      expect(toasts).toHaveLength(1);
      expect(loadComposerDraft("a")?.text).toBe("something I am writing");

      // And nothing was lost: once the box is empty again the same key hands the
      // sentence back, which is only true if it was still in the queue.
      saveComposerDraft("a", { attachments: [], text: "" });
      press({ code: "KeyX", key: "x", ctrlKey: !isMacOS, metaKey: isMacOS, shiftKey: true });
      expect(loadComposerDraft("a")?.text).toBe("held sentence");
    }
    finally {
      dispose();
    }
  });
});

describe("what the panel reports back", () => {
  it("leaves the grid for the reading layer when a cell says its editor was left", () => {
    // A grid cell draws the same `AgentThread` in its reading chrome, and the editor
    // there reports being left the same way. The panel's answer is the reading layer,
    // which is where a reader who closed an empty box wants to be.
    mount({ threads: [thread("a", 100)] });
    const layer = () => container.querySelector("[data-vibe-layer]")?.getAttribute("data-vibe-layer");
    expect(layer()).toBe("grid");

    const probe = container.querySelector<HTMLElement>("[data-leave-input-probe]");
    expect(probe, "a cell that can report being left").not.toBeNull();
    act(() => probe!.click());

    expect(layer()).toBe("read");
  });

  it("answers an approval raised in the input layer too", async () => {
    // The same two callbacks are wired into the input layer's view; a decision made while
    // dictating must close the card on the conversation just as one made while reading.
    const onApprovalDecided = vi.fn();
    h.approvals.activeApproval = { id: "ap_input" };
    mount({ threads: [thread("a", 100)], view: { onApprovalDecided } });
    press({ code: "Enter", key: "Enter" });
    expect(drawn("input")).toEqual(["a"]);

    const probe = container.querySelector<HTMLElement>("[data-leave-input-probe]");
    // The input layer's own probe belongs to the editor there; the approval probe is
    // beside it, in the same view.
    const approvalProbe = [...container.querySelectorAll<HTMLElement>("[data-approval-probe]")];
    expect(approvalProbe.length).toBeGreaterThan(0);
    await act(async () => {
      approvalProbe[0]!.click();
    });

    // The decision reaches the panel's queue, and the conversation is told afterwards.
    // (The input layer hands the composer the callback without the card's object — the
    // card itself is the composer's, so its exact argument is asserted in the reading
    // layer's case above.)
    expect(h.approvals.decideApproval).toHaveBeenCalledTimes(1);
    expect(h.approvals.decideApproval.mock.calls[0]![1]).toBe("approved");
    expect(onApprovalDecided).toHaveBeenCalledTimes(1);
    h.approvals.activeApproval = null;
    expect(probe).not.toBeNull();
  });

  it("opens a host's conversation in the shell from the reading layer's transcript", () => {
    const onOpenRemoteInShell = vi.fn();
    mount({
      catalogs: [{ desktopId: "desk_1", sessions: [{ pinned: true, sessionId: "s1", status: "completed", title: "Remote" }] } as never],
      onOpenRemoteInShell,
      peers: [{ desktopId: "desk_1", name: "Studio" } as never],
      threads: [],
    });
    press({ code: "Enter", key: "Enter" });

    act(() => {
      container.querySelector<HTMLElement>("[data-vibe-transcript-for]")!
        .dispatchEvent(new MouseEvent("dblclick", { bubbles: true }));
    });

    expect(onOpenRemoteInShell).toHaveBeenCalledWith("desk_1", "s1");
  });

  it("closes the tile's menu from its own dismiss", () => {
    mount({ threads: [thread("a", 100), thread("b", 200), thread("c", 300)] });
    press({ code: "Digit4", key: "4" });
    press({ code: "Enter", key: "Enter" });
    expect(container.querySelector("[data-vibe-menu-row=\"add:new\"]")).not.toBeNull();

    press({ code: "Escape", key: "Escape" });

    expect(container.querySelector("[role=\"menu\"]")).toBeNull();
  });
});
