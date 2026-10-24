// @vitest-environment jsdom
import type { RemoteApproval } from "./useRemoteApprovals";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

/**
 * A conversation on a machine another paired device holds.
 *
 * What the reader must be able to tell from inside the conversation: that the
 * machine is someone else's right now, that nothing written here would arrive,
 * and how to take it back — with the writes that the host would refuse closed
 * before they are sent, rather than reported afterwards as a transport error
 * (`superseded`).
 */
const h = vi.hoisted(() => ({
  blocks: [] as Array<Record<string, unknown>>,
  visibleSession: vi.fn(async () => {}),
}));

vi.mock("./remotePeerClient", () => ({
  setRemotePeerVisibleSession: h.visibleSession,
}));
vi.mock("./remoteToolTarget", () => ({ fetchRemoteToolTarget: vi.fn(async () => null) }));
vi.mock("../agent/MessageBlock", () => ({
  MessageBlock: (props: Record<string, unknown>) => {
    h.blocks.push(props);
    return <div data-block={(props.message as { id: string }).id} />;
  },
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

/** The host, as the peer list reports it while another device drives it. */
const heldPeer = {
  agentAvailable: true,
  bridgeInstanceId: "bridge_1",
  connected: true,
  control: "standby" as const,
  desktopId: "desk_1",
  error: null,
  features: [],
  holder: { name: "Hui's phone" },
  icon: "laptop",
  name: "Studio iMac",
  pairId: "pair_1",
  stopped: false,
};

const approval: RemoteApproval = {
  id: "ap1",
  riskLevel: null,
  sessionId: "sess_1",
  summary: "rm -rf build",
  title: "Run a shell command",
  toolName: "shell",
};

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
    onTakeControl: vi.fn(),
    onToggleLeftPanel: vi.fn(),
    peer: heldPeer,
    persistedEntryIds: new Set<string>(),
    sessionId: "sess_1",
    streaming: false,
    title: "Deadlock in the revive pass",
    workspaceName: null,
    ...over,
  };
}

async function render(over: Record<string, unknown> = {}) {
  const { RemoteConversationView: View } = await import("./RemoteConversationView");
  await act(async () => root.render(<View {...(props(over) as unknown as Parameters<typeof View>[0])} />));
}

/** The last block drawn, which is the settled turn these tests act on. */
function lastBlock(): Record<string, unknown> {
  return h.blocks[h.blocks.length - 1]!;
}

function button(label: string): HTMLButtonElement | null {
  return [...container.querySelectorAll("button")]
    .find(node => node.textContent === label || node.getAttribute("aria-label") === label) ?? null;
}

/** A user turn and the reply it produced: one settled point to fork at. */
const settledTurn = [
  { content: "run it", id: "m1", role: "user", sourceEntryId: "e1", status: "complete" },
  { content: "done", id: "m2", role: "assistant", status: "complete" },
];

beforeEach(() => {
  h.blocks = [];
  h.visibleSession.mockClear();
  container = document.body.appendChild(document.createElement("div"));
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

describe("a conversation on a machine another device holds", () => {
  it("says who holds it, and that nothing written here will arrive", async () => {
    await render();
    // The machine's name, the fact, and the consequence — the three things the
    // reader of a composer needs before they type.
    expect(container.textContent).toContain("Taken over by Hui's phone");
    expect(container.textContent).toContain("will not arrive");
  });

  it("draws no one's name when the holder never declared one", async () => {
    await render({ peer: { ...heldPeer, holder: undefined } });
    expect(container.textContent).toContain("Taken over by another device");
  });

  it("offers the way back, and hands the act to the caller", async () => {
    const onTakeControl = vi.fn(async () => {});
    await render({ onTakeControl });
    const take = button("Take over");
    expect(take).not.toBeNull();
    await act(async () => take!.click());
    expect(onTakeControl).toHaveBeenCalledTimes(1);
  });

  it("disables the control while the takeover is in flight", async () => {
    let release: () => void = () => {};
    const onTakeControl = vi.fn(() => new Promise<void>((resolve) => {
      release = resolve;
    }));
    await render({ onTakeControl });
    await act(async () => button("Take over")!.click());
    expect(button("Taking over…")?.disabled).toBe(true);
    await act(async () => release());
    expect(button("Take over")?.disabled).toBe(false);
  });

  it("says nothing on a machine this computer drives", async () => {
    await render({ peer: { ...heldPeer, control: "active" } });
    expect(container.textContent).not.toContain("Taken over by");
    expect(button("Take over")).toBeNull();
  });

  it("says nothing on a host too old to report who drives it", async () => {
    await render({ peer: { ...heldPeer, control: undefined } });
    expect(container.textContent).not.toContain("Taken over by");
  });

  it("keeps the banner out of a read-only panel cell, which has nothing to send", async () => {
    await render({ chrome: "read" });
    expect(container.textContent).not.toContain("Taken over by Hui's phone");
  });
});

describe("the writes it closes", () => {
  it("refuses an approval with the reason, rather than letting the host refuse it", async () => {
    const onDecideApproval = vi.fn();
    await render({ approvals: [approval], onDecideApproval });
    expect(container.textContent).toContain("This approval cannot be answered from here");
    const allow = button("Allow")!;
    const deny = button("Deny")!;
    expect(allow.disabled).toBe(true);
    expect(deny.disabled).toBe(true);
    // A disabled control receives no click; the assertion is that nothing would
    // have been sent even if it did.
    await act(async () => {
      allow.click();
      deny.click();
    });
    expect(onDecideApproval).not.toHaveBeenCalled();
  });

  it("still lets an approval be answered on a machine this computer drives", async () => {
    const onDecideApproval = vi.fn();
    await render({ approvals: [approval], onDecideApproval, peer: { ...heldPeer, control: "active" } });
    expect(container.textContent).not.toContain("This approval cannot be answered from here");
    await act(async () => button("Allow")!.click());
    expect(onDecideApproval).toHaveBeenCalledWith(approval, "allow");
  });

  it("draws no fork on a settled turn while another device holds the machine", async () => {
    await render({ messages: settledTurn });
    expect(lastBlock().onFork).toBeUndefined();
  });

  it("keeps the fork on the same turn when this computer drives the machine", async () => {
    await render({ messages: settledTurn, peer: { ...heldPeer, control: "active" } });
    expect(lastBlock().onFork).toBeTypeOf("function");
  });

  it("draws no continue on a failed run while another device holds the machine", async () => {
    await render({
      messages: [
        { content: "run it", id: "m1", role: "user", sourceEntryId: "e1", status: "complete" },
        { content: "", id: "m2", role: "assistant", runId: "run_1", status: "failed" },
      ],
    });
    expect(lastBlock().onContinue).toBeUndefined();
  });
});
