// @vitest-environment jsdom
import type { RemoteConversationView } from "./RemoteConversationView";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

/**
 * What `chrome` and `selectiveEvents` change — the two props the Vibe panel added
 * to this view, and the defaults that keep the main window exactly as it was.
 *
 * The panel's invariants ride on these two: `read` mounts no composer (so a grid of
 * cells leaves one editable element in the app), and a cell must **not** narrow the
 * host's event lane (several cells of one host are on screen at once, and the lane is
 * a one-conversation bargain).
 */
const h = vi.hoisted(() => ({
  visibleSession: vi.fn(async () => {}),
}));

vi.mock("./remotePeerClient", () => ({
  setRemotePeerVisibleSession: h.visibleSession,
}));
vi.mock("./remoteToolTarget", () => ({ fetchRemoteToolTarget: vi.fn(async () => null) }));
vi.mock("../agent/MessageBlock", () => ({
  MessageBlock: (props: { message: { id: string } }) => <div data-block={props.message.id} />,
}));
vi.mock("../agent/QuestionNavControl", () => ({
  QuestionNavControl: () => <div data-question-nav="" />,
}));
vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({
    isFullscreen: async () => false,
    onResized: async () => () => {},
    setFullscreen: async () => {},
  }),
}));
vi.mock("../agent/useStickyAutoScroll", () => ({
  useStickyAutoScroll: () => ({
    handleScroll: vi.fn(),
    preserveViewport: vi.fn(),
    scrollToLatest: vi.fn(),
    showJumpToLatest: false,
  }),
}));
vi.mock("../agent/useQuestionNav", () => ({
  useQuestionNav: () => ({
    goToNext: vi.fn(),
    goToPrevious: vi.fn(),
    hasNext: false,
    hasPrevious: false,
  }),
}));
vi.mock("../agent/useComposerInset", () => ({
  useComposerInset: () => ({ composerHeight: 120, composerRef: { current: null } }),
}));

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

let container: HTMLDivElement;
let root: ReturnType<typeof createRoot>;

function props(over: Record<string, unknown> = {}) {
  return {
    approvals: [],
    approvalErrors: {},
    approvalPending: null,
    composer: <div data-composer="" />,
    desktopId: "desk_1",
    error: null,
    hasMore: false,
    leftPanelExpanded: true,
    liveReplies: [],
    loading: false,
    loadingOlder: false,
    messages: [],
    olderHistoryFailed: false,
    onContinueRun: vi.fn(),
    onDecideApproval: vi.fn(),
    onFork: vi.fn(),
    onLoadOlder: vi.fn(),
    onOpenUsage: vi.fn(),
    onRetry: vi.fn(),
    onToggleLeftPanel: vi.fn(),
    peer: undefined,
    persistedEntryIds: new Set<string>(),
    sessionId: "sess_1",
    streaming: false,
    title: "Remote conversation",
    workspaceName: null,
    ...over,
  };
}

async function render(over: Record<string, unknown> = {}) {
  // The component is imported lazily so each test sees the module mocks above.
  const { RemoteConversationView: View } = await import("./RemoteConversationView");
  await act(async () => root.render(<View {...(props(over) as unknown as Parameters<typeof RemoteConversationView>[0])} />));
}

beforeEach(() => {
  h.visibleSession.mockClear();
  container = document.body.appendChild(document.createElement("div"));
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

describe("chrome: full (the default)", () => {
  it("draws the conversation's own title bar and composer", async () => {
    await render();
    expect(container.textContent).toContain("Remote conversation");
    expect(container.querySelector("[data-composer]")).not.toBeNull();
  });

  it("narrows the host's event lane to this conversation", async () => {
    await render();
    expect(h.visibleSession).toHaveBeenCalledWith("desk_1", "sess_1");
  });
});

describe("chrome: read (one cell of the panel)", () => {
  it("drops the title bar — the cell has its own — and keeps the transcript", async () => {
    await render({
      chrome: "read",
      messages: [{ content: "hi", id: "m1", role: "user", status: "complete" }],
    });
    expect(container.textContent).not.toContain("Remote conversation");
    expect(container.querySelector("[data-block=\"m1\"]")).not.toBeNull();
  });

  it("draws no composer, which is what keeps the panel to one editor", async () => {
    await render({ chrome: "read" });
    expect(container.querySelector("[data-composer]")).toBeNull();
    // No composer means no reserved height for one either.
    expect(container.querySelector("[data-question-nav]")).toBeNull();
  });

  it("still shows an approval that needs an answer", async () => {
    // The one thing a cell must be able to ask for without a composer: a decision
    // the host is blocked on. Its own card, in a strip at the foot.
    await render({
      approvals: [{
        id: "ap1",
        riskLevel: null,
        sessionId: "sess_1",
        summary: "rm -rf build",
        title: "Run a shell command",
        toolName: "shell",
      }],
      chrome: "read",
    });
    expect(container.textContent).toContain("Run a shell command");
    // …and not the band's other furniture, which belongs to a conversation that can
    // be answered from here.
    expect(container.querySelector("[data-composer]")).toBeNull();
  });

  it("draws no strip at all when nothing is pending", async () => {
    await render({ chrome: "read" });
    expect(container.textContent).not.toContain("Run a shell command");
  });

  it("does not narrow the host's event lane", async () => {
    // The panel shows several conversations of one host; narrowing would route the
    // others' events away from the cells that are drawing them.
    await render({ chrome: "read", selectiveEvents: false });
    expect(h.visibleSession).not.toHaveBeenCalled();
  });
});
