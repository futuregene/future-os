// @vitest-environment jsdom
import type { StoredThread, StoredWorkspace } from "../../integrations/storage/threadStore";
import type { VibeThreadViewProps } from "./VibeStage";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { onFutureEvent } from "../../lib/futureEvents";
import { installShortcuts } from "../../test/installShortcuts";
import { clearCommandHandlers } from "../keyboard/commands";
import { resetShortcutScopes } from "../keyboard/scopes";
import { resetAllCommandBindings } from "../keyboard/shortcuts";
import { clearVibePreferences, readVibePreferences } from "./vibePreferences";
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
  approvals: { activeApproval: null as unknown, decideApproval: vi.fn(async () => {}) },
  counts: new Map<string, number>(),
  cached: { current: null as { model?: string; thinkingLevel?: string } | null },
}));

vi.mock("../agent/AgentThread", () => ({
  AgentThread: (props: { chrome?: string; thread: { id: string } }) => (
    <div data-chrome={props.chrome ?? "full"} data-thread={props.thread.id}>
      {props.chrome === "input"
        ? <div contentEditable data-input-editor="" suppressContentEditableWarning />
        : null}
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

vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({
    isFullscreen: async () => false,
    setFullscreen: async () => {},
  }),
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

function mount(options: {
  threads: StoredThread[];
  onClose?: () => void;
  onOpenInShell?: (thread: StoredThread) => void;
  onTogglePin?: (thread: StoredThread) => void;
}) {
  act(() => {
    root.render(
      <VibeStage
        onClose={options.onClose ?? (() => {})}
        onOpenInShell={options.onOpenInShell ?? (() => {})}
        onTogglePin={options.onTogglePin ?? (() => {})}
        runStatuses={{}}
        threads={options.threads}
        unreadThreadIds={new Set()}
        view={VIEW}
        workspaces={WORKSPACES}
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

beforeEach(() => {
  clearVibePreferences();
  resetAllCommandBindings();
  resetShortcutScopes();
  clearCommandHandlers();
  h.counts = new Map();
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
    // The first row is the layout choice; → moves it off "automatic".
    press({ code: "ArrowRight", key: "ArrowRight" });
    expect(readVibePreferences().columns).toBe(1);
    // …and the panel says so, so the reader can see what they just chose: three
    // cells in one column, marked as a manual choice.
    expect(container.textContent).toContain("1×3");
  });

  it("unpins the focused cell from the menu", () => {
    const onTogglePin = vi.fn();
    mount({ onTogglePin, threads: [thread("a", 100), thread("b", 200)] });
    press({ code: "ContextMenu", key: "ContextMenu" });
    // Walk to "remove". The walk skips the row that cannot run (the focused cell
    // is already first, so "move earlier" is disabled), which is why this counts
    // steps by what is selected rather than by a fixed number of presses.
    const selected = () => container.querySelector("[aria-current=\"true\"]")?.getAttribute("data-vibe-menu-row");
    for (let step = 0; step < 8 && selected() !== "remove"; step++)
      press({ code: "ArrowDown", key: "ArrowDown" });
    expect(selected()).toBe("remove");
    press({ code: "Enter", key: "Enter" });
    expect(onTogglePin).toHaveBeenCalledWith(expect.objectContaining({ id: "a" }));
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
    act(() => {
      root.render(
        <VibeStage
          onClose={() => {}}
          onOpenInShell={() => {}}
          onTogglePin={() => {}}
          runStatuses={runStatuses as never}
          threads={[thread("a", 100), thread("b", 200), thread("c", 300)]}
          unreadThreadIds={new Set()}
          view={VIEW}
          workspaces={WORKSPACES}
        />,
      );
    });
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
    expect(container.querySelector("[aria-current=\"true\"]")?.getAttribute("data-vibe-menu-row")).toBe("attention:b");
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
    const onTogglePin = vi.fn();
    mount({ onTogglePin, threads: [thread("a", 100), thread("b", 200)] });
    press({ code: "Digit2", key: "2" });
    press({ code: "Backspace", key: "Backspace", ctrlKey: true, shiftKey: true });
    expect(onTogglePin).toHaveBeenCalledWith(expect.objectContaining({ id: "b" }));
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
    act(() => {
      root.render(
        <VibeStage
          onClose={() => {}}
          onOpenInShell={() => {}}
          onTogglePin={() => {}}
          runStatuses={{}}
          threads={[thread("a", 100), thread("b", 50), thread("c", 300)]}
          unreadThreadIds={new Set()}
          view={VIEW}
          workspaces={WORKSPACES}
        />,
      );
    });
    expect(drawn("read")).toEqual(["b", "c", "a"]);
  });

  it("drops a conversation that is unpinned elsewhere without moving the rest", () => {
    mount({ threads: [thread("a", 100), thread("b", 200), thread("c", 300)] });
    press({ code: "Digit3", key: "3" });
    press({ code: "ArrowLeft", key: "ArrowLeft", ctrlKey: true, altKey: true });
    expect(drawn("read")).toEqual(["a", "c", "b"]);
    act(() => {
      root.render(
        <VibeStage
          onClose={() => {}}
          onOpenInShell={() => {}}
          onTogglePin={() => {}}
          runStatuses={{}}
          threads={[thread("b", 200), thread("c", 300)]}
          unreadThreadIds={new Set()}
          view={VIEW}
          workspaces={WORKSPACES}
        />,
      );
    });
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
