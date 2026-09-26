import type { AgentMessage } from "@future-os/thread-projection";
// @vitest-environment jsdom
import type { ReactNode } from "react";
import type { Root } from "react-dom/client";
import type { AgentConnectionState } from "../../components/layout/AppShell";
import type {
  StoredApprovalRequest,
  StoredThread,
} from "../../integrations/storage/threadStore";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { onFutureEvent } from "../../lib/futureEvents";
import { AgentThread } from "./AgentThread";

/**
 * Every child is stubbed: this file is about `AgentThread`'s own composition —
 * which notice it shows for which connection state, what it hands the composer,
 * and how the recovery / fork / compaction callbacks resolve. The children have
 * their own behaviour tests elsewhere.
 */
const h = vi.hoisted(() => ({
  state: {
    handleAbort: vi.fn(async () => {}),
    handleSend: vi.fn<(payload: unknown, onAccepted?: () => void) => Promise<void>>(async () => {}),
    retryHistory: vi.fn(async () => {}),
  },
  paging: {
    coolingDown: false,
    handleScroll: vi.fn(),
    loadOlder: vi.fn(),
    prepareSearch: vi.fn(async () => {}),
    revealSearchMatch: vi.fn(),
    scrollToLatest: vi.fn(),
    showJumpToLatest: false,
    showLoadOlderHint: false,
    visibleMessages: [] as AgentMessage[],
  },
  scrollbar: {
    handleScroll: vi.fn(),
    handleThumbPointerDown: vi.fn(),
    scrollbar: null,
    updateFloatingScrollbar: vi.fn(),
  },
  reco: { dismiss: vi.fn(), evaluate: vi.fn(async () => null) },
  agentState: { current: null as { isCompacting?: boolean; usage?: unknown } | null },
  store: { forkThread: vi.fn() },
  client: { compactThreadContext: vi.fn() },
  skills: { installRecommendedSkill: vi.fn(async () => {}) },
  /** The hook results the component reads; each test rewrites what it needs. */
  threadState: { value: {} as Record<string, unknown> },
  /** The arguments `AgentThread` handed `useMessagePaging`, for callback tests. */
  pagingArgs: { value: {} as Record<string, unknown> },
  /** Observes the promise the composer awaits from `onSend`. */
  sendProbe: { rejected: false, settled: false },
}));

vi.mock("./ThreadHeader", () => ({
  ThreadHeader: (props: { action?: ReactNode }) => <div data-header="">{props.action ?? null}</div>,
}));
vi.mock("./ThreadSearch", () => ({ ThreadSearch: () => <div data-search="" /> }));
vi.mock("./Composer", () => ({
  Composer: (props: Record<string, unknown>) => (
    <div
      data-composer=""
      data-compaction={typeof props.onCompactContext === "function" ? "yes" : "no"}
      data-compacting={String(props.compactionInProgress)}
      data-disabled={String(props.disabled)}
      data-empty-reason={String(props.modelsEmptyReason)}
      data-sending={String(props.sending)}
    >
      <button data-abort="" onClick={() => (props.onAbort as () => void)()} type="button" />
      <button
        data-install=""
        onClick={() => (props.skillRecommendation as { onInstall: (card: { description: string; name: string }) => void }).onInstall({ description: "web", name: "future-web" })}
        type="button"
      />
      <button data-compact="" onClick={() => void (props.onCompactContext as (() => Promise<void>) | undefined)?.()} type="button" />
      <button
        data-send=""
        onClick={() => {
          (props.onSend as (p: unknown) => Promise<void>)({ attachments: [], content: "hi" }).then(
            () => {
              h.sendProbe.settled = true;
            },
            () => {
              h.sendProbe.rejected = true;
            },
          );
        }}
        type="button"
      />
    </div>
  ),
}));
vi.mock("./MessageList", () => ({
  MessageList: (props: {
    messages: AgentMessage[];
    onContinue: (m: AgentMessage) => void;
    onFork: (m: AgentMessage) => void;
    onRetry: (m: AgentMessage, source: AgentMessage) => void;
  }) => (
    <div data-count={props.messages.length} data-list="">
      {props.messages.length > 0
        ? (
            <>
              <button data-continue="" onClick={() => props.onContinue(props.messages[props.messages.length - 1]!)} type="button" />
              <button data-fork="" onClick={() => void props.onFork(props.messages[props.messages.length - 1]!)} type="button" />
              <button data-retry="" onClick={() => props.onRetry(props.messages[props.messages.length - 1]!, props.messages[0]!)} type="button" />
            </>
          )
        : null}
    </div>
  ),
}));
vi.mock("./ApprovalPrompt", () => ({
  ApprovalPrompt: (props: { threadMode?: string }) => (
    <div data-approval="" data-thread-mode={props.threadMode ?? ""} />
  ),
}));
vi.mock("../../components/ui/FloatingScrollbar", () => ({
  FloatingScrollbar: () => <div data-scrollbar="" />,
}));
vi.mock("./useAgentThreadState", () => ({ useAgentThreadState: () => h.threadState.value }));
vi.mock("./useMessagePaging", () => ({
  useMessagePaging: (options: Record<string, unknown>) => {
    h.pagingArgs.value = options;
    return h.paging;
  },
}));
vi.mock("../../lib/useFloatingScrollbar", () => ({
  useFloatingScrollbar: () => ({ ...h.scrollbar, scrollRef: { current: null } }),
}));
vi.mock("./useComposerInset", () => ({
  useComposerInset: () => ({ composerHeight: 0, composerRef: { current: null } }),
}));
vi.mock("./useSkillRecommendation", () => ({
  useSkillRecommendation: () => ({ candidates: [], state: { recommendation: null }, ...h.reco }),
}));
vi.mock("../../integrations/agent/agentStateCache", () => ({
  useCachedAgentState: () => h.agentState.current,
}));
vi.mock("../../integrations/storage/threadStore", () => ({ forkThread: h.store.forkThread }));
vi.mock("../../integrations/agent/agentClient", () => ({
  compactThreadContext: h.client.compactThreadContext,
}));
vi.mock("../skills/installRecommendedSkill", () => ({
  installRecommendedSkill: h.skills.installRecommendedSkill,
}));

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

const thread = { agentSessionId: "S1", id: "T1", mode: "workspace" } as StoredThread;
const ready: AgentConnectionState = { readiness: "ready", status: "connected" };

function user(id: string, over: Partial<AgentMessage> = {}): AgentMessage {
  return { content: `prompt ${id}`, id, role: "user", status: "complete", ...over } as AgentMessage;
}

function assistant(id: string, over: Partial<AgentMessage> = {}): AgentMessage {
  return { content: `answer ${id}`, id, role: "assistant", status: "complete", ...over } as AgentMessage;
}

let root: Root;
let container: HTMLDivElement;
let props: Parameters<typeof AgentThread>[0];

beforeEach(() => {
  vi.clearAllMocks();
  h.state.handleSend.mockResolvedValue(undefined);
  h.reco.evaluate.mockResolvedValue(null);
  h.agentState.current = null;
  h.store.forkThread.mockResolvedValue("T2");
  h.client.compactThreadContext.mockResolvedValue({ operationId: "op-1" });
  h.sendProbe.rejected = false;
  h.sendProbe.settled = false;
  h.paging.coolingDown = false;
  h.paging.showJumpToLatest = false;
  h.paging.showLoadOlderHint = false;
  h.paging.visibleMessages = [user("u1"), assistant("a1")];

  h.threadState.value = {
    handleAbort: h.state.handleAbort,
    handleSend: h.state.handleSend,
    historyError: null,
    loadAllHistoryForSearch: undefined,
    loadOlderHistory: undefined,
    loadingIndicator: false,
    loadingThread: false,
    messages: [user("u1"), assistant("a1")],
    renderWorkspace: { workspaceId: "W1", workspacePath: "/work" },
    retryHistory: h.state.retryHistory,
    sessionChanged: false,
  };

  props = {
    agentConnection: ready,
    approvalTier: "manual",
    futureBalance: 10,
    futureSessionStatus: "authenticated",
    headerAction: null,
    leftPanelExpanded: true,
    loadingStore: false,
    modelId: "m1",
    modelOptions: [],
    onApprovalDecision: vi.fn(async () => {}),
    onChangeApprovalTier: vi.fn(),
    onForked: vi.fn(),
    onModelChange: vi.fn(),
    onOpenAccount: vi.fn(),
    onOpenModels: vi.fn(),
    onOpenProviders: vi.fn(),
    onPromptConsumed: vi.fn(),
    onRetryAgentConnection: vi.fn(),
    onThreadActivity: vi.fn(),
    onThinkingLevelChange: vi.fn(),
    onToggleLeftPanel: vi.fn(),
    pendingPrompt: null,
    skillRecommend: false,
    thinkingLevel: "high",
    thread,
  };
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

function render(over: Partial<Parameters<typeof AgentThread>[0]> = {}) {
  act(() => root.render(<AgentThread {...props} {...over} />));
}

/** Re-render after mutating what a mocked hook returns. */
function rerender(over: Partial<Parameters<typeof AgentThread>[0]> = {}) {
  render(over);
}

function setMessages(messages: AgentMessage[]) {
  // The paging hook decides what `MessageList` sees; keep them in step so a
  // callback receives the very object the transcript holds (identity matters
  // for forking, which locates the message with `indexOf`).
  h.threadState.value = { ...h.threadState.value, messages };
  h.paging.visibleMessages = messages;
}

async function click(selector: string) {
  await act(async () => {
    const element = container.querySelector<HTMLButtonElement>(selector);
    if (!element)
      throw new Error(`no element matching ${selector}`);
    element.click();
    await Promise.resolve();
  });
}

function composer() {
  return container.querySelector<HTMLElement>("[data-composer]")!;
}

/** The connection notice, located by its warning surface (it is internal). */
function notice(): HTMLElement | null {
  return container.querySelector<HTMLElement>("div[class*='bg-warning-soft']");
}

function noticeButtonLabel() {
  return notice()?.querySelector("button")?.textContent ?? null;
}

describe("agentThread composition", () => {
  it("shows the loading state, the no-thread state, or the message list", () => {
    render();
    expect(container.querySelector("[data-list]")?.getAttribute("data-count")).toBe("2");

    h.threadState.value = { ...h.threadState.value, loadingIndicator: true };
    rerender();
    expect(container.querySelector("[data-list]")).toBeNull();
    expect(container.textContent).toContain("Loading FutureOS thread...");

    h.threadState.value = { ...h.threadState.value, loadingIndicator: false };
    rerender({ thread: null });
    expect(container.textContent).toContain("No active thread.");
  });

  it("keeps the empty state from flashing while the store is still loading a thread", () => {
    // boundary: no thread yet AND the store has not settled — the shell must not
    // claim there is no conversation while the store may still produce one.
    render({ loadingStore: true, thread: null });
    expect(container.textContent).not.toContain("No active thread.");
    expect(composer().getAttribute("data-disabled")).toBe("true");

    // Once the store has settled with nothing, the empty state is the truth.
    rerender({ loadingStore: false, thread: null });
    expect(container.textContent).toContain("No active thread.");
  });

  it("renders the search affordance only for an open conversation", () => {
    render();
    expect(container.querySelector("[data-search]")).not.toBeNull();

    rerender({ thread: null });
    expect(container.querySelector("[data-search]")).toBeNull();
  });

  it("reports a session change and a history failure with a retry", async () => {
    render();
    expect(container.querySelector("[role=status]")).toBeNull();
    expect(container.querySelector("[role=alert]")).toBeNull();

    h.threadState.value = { ...h.threadState.value, historyError: "history is unreadable", sessionChanged: true };
    rerender();
    expect(container.querySelector("[role=status]")?.textContent).toContain("background session changed");
    expect(container.querySelector("[role=alert]")?.textContent).toContain("history is unreadable");

    await click("[role=alert] button");
    expect(h.state.retryHistory).toHaveBeenCalledTimes(1);
  });

  it("offers load-older while history remains and disables it during the cooldown", async () => {
    render();
    expect(container.querySelector("[aria-label='Load earlier messages']")).toBeNull();

    h.paging.showLoadOlderHint = true;
    rerender();
    expect(container.querySelector<HTMLButtonElement>("[aria-label='Load earlier messages']")!.disabled).toBe(false);
    await click("[aria-label='Load earlier messages']");
    expect(h.paging.loadOlder).toHaveBeenCalledTimes(1);

    // concurrency: a second collision while the first is still in flight must
    // not be clickable.
    h.paging.coolingDown = true;
    rerender();
    const busy = container.querySelector<HTMLButtonElement>("[aria-label='Load earlier messages']")!;
    expect(busy.disabled).toBe(true);
    expect(busy.getAttribute("aria-busy")).toBe("true");
  });

  it("offers jump-to-latest once the reader is far from the bottom", async () => {
    render();
    expect(container.querySelector("[aria-label='Jump to latest']")).toBeNull();

    h.paging.showJumpToLatest = true;
    rerender();
    await click("[aria-label='Jump to latest']");
    expect(h.paging.scrollToLatest).toHaveBeenCalledTimes(1);
  });

  it("mounts the approval prompt with this thread's mode", () => {
    render();
    expect(container.querySelector("[data-approval]")).toBeNull();

    const approval = { id: "a1", threadId: "T1", toolName: "shell" } as unknown as StoredApprovalRequest;
    rerender({ activeApproval: approval });
    expect(container.querySelector("[data-approval]")?.getAttribute("data-thread-mode")).toBe("workspace");
  });
});

describe("agentThread composer wiring", () => {
  it("passes the compaction flag through, and never invents one with no agent state", () => {
    // A mutation study found this wiring untested: replacing `?? false` with
    // `?? true` left all 51 tests green, because nothing observed the prop. The
    // `agentState = null` case is the discriminating one — the fallback must be
    // "not compacting", or a thread whose agent state has not loaded yet would
    // show the composer as mid-compaction and withdraw the compact affordance.
    h.agentState.current = { isCompacting: true };
    render();
    expect(composer().getAttribute("data-compacting")).toBe("true");

    h.agentState.current = { isCompacting: false };
    rerender();
    expect(composer().getAttribute("data-compacting")).toBe("false");

    h.agentState.current = null;
    rerender();
    expect(composer().getAttribute("data-compacting")).toBe("false");
  });

  it("marks a run in flight only from a trailing streaming assistant bubble", () => {
    render();
    expect(composer().getAttribute("data-sending")).toBe("false");

    setMessages([user("u1"), assistant("a1", { status: "streaming" })]);
    rerender();
    expect(composer().getAttribute("data-sending")).toBe("true");

    // boundary: a streaming bubble BEFORE the last user message is an older
    // turn's leftover, not this turn's run.
    setMessages([assistant("old", { status: "streaming" }), user("u2")]);
    rerender();
    expect(composer().getAttribute("data-sending")).toBe("false");
  });

  it("disables the composer with no thread, while loading, or while the store loads", () => {
    render();
    expect(composer().getAttribute("data-disabled")).toBe("false");

    // `loadingThread` is the history load's own flag, not a prop.
    h.threadState.value = { ...h.threadState.value, loadingThread: true };
    rerender();
    expect(composer().getAttribute("data-disabled")).toBe("true");

    h.threadState.value = { ...h.threadState.value, loadingThread: false };
    rerender({ loadingStore: true });
    expect(composer().getAttribute("data-disabled")).toBe("true");

    rerender({ loadingStore: false, thread: null });
    expect(composer().getAttribute("data-disabled")).toBe("true");
  });

  it("explains an empty model picker differently for an all-disabled provider set", () => {
    render();
    expect(composer().getAttribute("data-empty-reason")).toBe("no_models");

    rerender({ agentConnection: { readiness: "all_disabled", status: "connected" } });
    expect(composer().getAttribute("data-empty-reason")).toBe("all_disabled");
  });

  it("settles the composer's send early once the pipeline accepts the prompt", async () => {
    // The composer waits for *delivery*, not for the whole run: `handleSend`
    // resolves the outer promise as soon as the agent accepts the prompt.
    let accepted: (() => void) | undefined;
    let release: () => void = () => {};
    h.state.handleSend.mockImplementation(async (_payload: unknown, onAccepted?: () => void) => {
      accepted = onAccepted;
      await new Promise<void>((resolve) => {
        release = resolve;
      });
    });
    render();

    await click("[data-send]");
    expect(h.state.handleSend).toHaveBeenCalledWith({ attachments: [], content: "hi" }, expect.any(Function));
    expect(h.sendProbe.settled).toBe(false);

    // The agent accepted: the composer unlocks while the run keeps streaming.
    accepted?.();
    await act(async () => {
      await Promise.resolve();
    });
    expect(h.sendProbe.settled).toBe(true);

    release();
    await act(async () => {
      await Promise.resolve();
    });
  });

  it("settles the composer's send when the pipeline resolves on its own", async () => {
    let release: () => void = () => {};
    h.state.handleSend.mockImplementation(async () => {
      await new Promise<void>((resolve) => {
        release = resolve;
      });
    });
    render();

    await click("[data-send]");
    expect(h.sendProbe.settled).toBe(false);

    release();
    await act(async () => {
      await Promise.resolve();
    });
    expect(h.sendProbe.settled).toBe(true);
  });

  it("rejects the composer's send when the pipeline refuses the prompt", async () => {
    // error-path: an "already running" refusal must reach the composer instead of
    // leaving the box silently locked.
    h.state.handleSend.mockRejectedValue(new Error("already running"));
    render();

    await click("[data-send]");
    expect(h.sendProbe.rejected).toBe(true);
    expect(h.sendProbe.settled).toBe(false);
  });

  it("forwards an abort to the run owner", async () => {
    render();
    await click("[data-abort]");
    expect(h.state.handleAbort).toHaveBeenCalledTimes(1);
  });

  it("exposes context compaction only for a session that can compact", () => {
    render();
    expect(composer().getAttribute("data-compaction")).toBe("yes");

    rerender({ thread: { ...thread, agentSessionId: null } as StoredThread });
    expect(composer().getAttribute("data-compaction")).toBe("no");
  });

  it("installs the recommended skill through the skills feature", async () => {
    render();
    await click("[data-install]");
    expect(h.skills.installRecommendedSkill).toHaveBeenCalledWith("future-web");
  });

  it("settles the scrollbar after content lands and tracks its own scroll", () => {
    render();
    // The paging hook owns the settled signal; the shell's floating scrollbar
    // must be re-measured both when content settles and while scrolling.
    (h.pagingArgs.value.onContentSettled as () => void)();
    expect(h.scrollbar.updateFloatingScrollbar).toHaveBeenCalledWith(false);

    (h.pagingArgs.value.onScroll as () => void)();
    expect(h.scrollbar.handleScroll).toHaveBeenCalledTimes(1);
    // The follow gate is the loading indicator, inverted.
    expect(h.pagingArgs.value.followEnabled).toBe(true);
    expect(h.pagingArgs.value.userExchangeCount).toBe(10);
  });
});

describe("agentThread recovery callbacks", () => {
  it("continues a failed message with the recovery prompt", async () => {
    render();
    await click("[data-continue]");
    expect(h.state.handleSend).toHaveBeenCalledWith({
      attachments: [],
      content: "继续上一个任务。\n\n上一条失败消息摘要:\nanswer a1",
    });
  });

  it("retries from the user message that prompted the reply, keeping its attachments", async () => {
    setMessages([
      user("u1", { attachments: [{ id: "att-1" }] as unknown as AgentMessage["attachments"] }),
      assistant("a1"),
    ]);
    rerender();
    await click("[data-retry]");

    expect(h.state.handleSend).toHaveBeenCalledWith({
      attachments: [{ id: "att-1" }],
      content: "prompt u1",
    });
  });

  it("retries a message that carries no attachments with an empty list", async () => {
    // boundary: `attachments` is OPTIONAL on `AgentMessage`, so the ordinary
    // retry - a plain prompt with nothing attached - has `undefined` here. The
    // sibling test above is the only one that drives this callback, and its name
    // says it: it always supplies attachments, so the `?? []` fallback had never
    // been exercised. Asserting `toHaveBeenCalledWith({ attachments: [] })` is not
    // enough on its own, because an argument object with `attachments: undefined`
    // satisfies it under structural equality - hence the explicit `Array.isArray`,
    // which is what actually fails when the fallback is removed.
    setMessages([user("u1"), assistant("a1")]);
    rerender();
    await click("[data-retry]");

    expect(h.state.handleSend).toHaveBeenCalledTimes(1);
    const [payload] = h.state.handleSend.mock.calls[0]! as [{ attachments?: unknown; content: string }];
    expect(payload.content).toBe("prompt u1");
    expect(Array.isArray(payload.attachments)).toBe(true);
    expect(payload.attachments).toEqual([]);
  });

  it("answers a recover-run retry with the named trigger message", async () => {
    render();
    await act(async () => {
      window.dispatchEvent(new CustomEvent("futureos:recover-run", {
        detail: { action: "retry", runId: "R9", triggerMessageId: "u1" },
      }));
      await Promise.resolve();
    });
    expect(h.state.handleSend).toHaveBeenCalledWith({ attachments: [], content: "prompt u1" });
  });

  it("does not resend a different message when the trigger id is unknown", async () => {
    // boundary/error-path: a stale trigger id must not silently retry whichever
    // message happens to precede the run's reply.
    render();
    await act(async () => {
      window.dispatchEvent(new CustomEvent("futureos:recover-run", {
        detail: { action: "retry", runId: "R9", triggerMessageId: "gone" },
      }));
      await Promise.resolve();
    });
    expect(h.state.handleSend).not.toHaveBeenCalled();
  });

  it("retries the run's own user message when no trigger id is given", async () => {
    render();
    await act(async () => {
      window.dispatchEvent(new CustomEvent("futureos:recover-run", {
        detail: { action: "retry", runId: "R9" },
      }));
      await Promise.resolve();
    });
    expect(h.state.handleSend).toHaveBeenCalledWith({ attachments: [], content: "prompt u1" });
  });

  it("does nothing for a retry whose history has no user message", async () => {
    // boundary: an assistant-only transcript has nothing to resend.
    setMessages([assistant("a1")]);
    rerender();
    await act(async () => {
      window.dispatchEvent(new CustomEvent("futureos:recover-run", {
        detail: { action: "retry", runId: "R9" },
      }));
      await Promise.resolve();
    });
    expect(h.state.handleSend).not.toHaveBeenCalled();
  });

  it("continues a run by loading its resume summary into the prompt", async () => {
    render();
    await act(async () => {
      window.dispatchEvent(new CustomEvent("futureos:recover-run", {
        detail: { action: "continue", runId: "R9" },
      }));
      await Promise.resolve();
    });
    expect(h.state.handleSend).toHaveBeenCalledTimes(1);
    expect((h.state.handleSend.mock.calls[0] as unknown[])[0]).toMatchObject({
      attachments: [],
      content: expect.stringContaining("继续上一个任务。"),
    });
  });
});

describe("agentThread forking", () => {
  it("forks from the user message that produced the reply and reports the new thread", async () => {
    setMessages([user("u1", { sourceEntryId: "entry-1" }), assistant("a1")]);
    rerender();
    await click("[data-fork]");

    expect(h.store.forkThread).toHaveBeenCalledWith("T1", "entry-1", expect.stringMatching(/^desktop-fork:/));
    expect(props.onForked).toHaveBeenCalledWith("T2");
  });

  it("reuses one idempotency key per fork intent across a failed retry", async () => {
    // concurrency: a double-click (or a UI retry after a failure) must not open
    // two branches from the same fork point.
    setMessages([user("u1", { sourceEntryId: "entry-1" }), assistant("a1")]);
    rerender();
    h.store.forkThread.mockRejectedValueOnce(new Error("backend down"));

    await click("[data-fork]");
    await click("[data-fork]");

    expect(h.store.forkThread).toHaveBeenCalledTimes(2);
    expect(h.store.forkThread.mock.calls[0]![2]).toBe(h.store.forkThread.mock.calls[1]![2]);
  });

  it("releases the idempotency key after a successful fork", async () => {
    setMessages([user("u1", { sourceEntryId: "entry-1" }), assistant("a1")]);
    rerender();
    await click("[data-fork]");
    await click("[data-fork]");

    // A deliberate second branch from the same message is a new intent.
    expect(h.store.forkThread.mock.calls[0]![2]).not.toBe(h.store.forkThread.mock.calls[1]![2]);
  });

  it("reports a fork failure with the backend's reason", async () => {
    const toasts: { message: string; tone?: string }[] = [];
    const off = onFutureEvent("toast", toast => void toasts.push(toast));
    setMessages([user("u1", { sourceEntryId: "entry-1" }), assistant("a1")]);
    rerender();
    h.store.forkThread.mockRejectedValue(new Error("no such entry"));

    await click("[data-fork]");

    expect(toasts).toEqual([{ message: "Fork failed: no such entry", tone: "error" }]);
    off();
  });

  it("refuses to fork a message that is not persisted yet", async () => {
    // error-path: a live streaming reply has no entry id to branch from.
    const toasts: { message: string }[] = [];
    const off = onFutureEvent("toast", toast => void toasts.push(toast));
    setMessages([user("u1"), assistant("a1", { status: "streaming" })]);
    rerender();

    await click("[data-fork]");

    expect(h.store.forkThread).not.toHaveBeenCalled();
    expect(toasts[0]?.message).toContain("not yet persisted");
    off();
  });

  it("walks back past an older reply to the user question that started the exchange", async () => {
    // boundary: the fork point is found by walking back from the selected reply
    // until a USER message is reached, so the walk must step over any earlier
    // assistant bubble. That happens for real after a retry - the thread holds the
    // failed reply and the new one - and every existing test uses a bare
    // [user, assistant] pair, so the first backward step always found the user
    // message and the "not a user message" arm never ran. Taking the immediate
    // predecessor instead would fork from the assistant bubble, which has no
    // `sourceEntryId`, and the fork would be refused as "not yet persisted".
    setMessages([
      user("u1", { sourceEntryId: "entry-1" }),
      assistant("a1", { status: "failed" }),
      assistant("a2"),
    ]);
    rerender();
    await click("[data-fork]");

    expect(h.store.forkThread).toHaveBeenCalledWith("T1", "entry-1", expect.stringMatching(/^desktop-fork:/));
    expect(props.onForked).toHaveBeenCalledWith("T2");
  });

  it("does nothing when there is no preceding user message to fork from", async () => {
    setMessages([assistant("a1")]);
    rerender();
    await click("[data-fork]");
    expect(h.store.forkThread).not.toHaveBeenCalled();
  });

  it("offers no fork affordance at all for an empty transcript", () => {
    setMessages([]);
    rerender();
    expect(container.querySelector("[data-list]")?.getAttribute("data-count")).toBe("0");
    expect(container.querySelector("[data-fork]")).toBeNull();
  });

  it("does not fork when no conversation is open", async () => {
    // The transcript can still be on screen from the cached snapshot while the
    // store has not produced a thread yet; there is nothing to branch from.
    rerender({ loadingStore: true, thread: null });
    await click("[data-fork]");
    expect(h.store.forkThread).not.toHaveBeenCalled();
  });
});

describe("agentThread compaction", () => {
  /** The agent event the backend emits when compaction reaches a terminal state. */
  function emitAgentEvent(over: Record<string, unknown> = {}) {
    window.dispatchEvent(new CustomEvent("future:agent-event", {
      detail: {
        eventType: "compaction_committed",
        payload: { operation_id: "op-1" },
        sessionId: "S1",
        threadId: "T1",
        ...over,
      },
    }));
  }

  async function flush() {
    await act(async () => {
      await Promise.resolve();
    });
  }

  it("waits for the matching committed event before settling", async () => {
    render();
    await click("[data-compact]");
    expect(h.client.compactThreadContext).toHaveBeenCalledWith("T1");

    emitAgentEvent();
    await flush();
    // A settled wait leaves no pending handle behind: the second compaction can
    // start immediately rather than being cancelled.
    await click("[data-compact]");
    expect(h.client.compactThreadContext).toHaveBeenCalledTimes(2);
  });

  it("replays a terminal event that arrived before the operation id was known", async () => {
    // concurrency: the backend can commit before `compactThreadContext` returns
    // the operation id, so the event is buffered and matched afterwards.
    let resolveCompact: (value: { operationId: string }) => void = () => {};
    h.client.compactThreadContext.mockImplementation(() => new Promise((resolve) => {
      resolveCompact = resolve;
    }));
    render();

    await act(async () => {
      container.querySelector<HTMLButtonElement>("[data-compact]")!.click();
      await Promise.resolve();
    });
    emitAgentEvent();
    resolveCompact({ operationId: "op-1" });
    await flush();

    // The buffered event matched, so the wait settled and a second compaction is
    // not blocked by the first one's cleanup.
    expect(h.client.compactThreadContext).toHaveBeenCalledTimes(1);
  });

  it("ignores events from another thread, session, shape, or operation", async () => {
    // boundary: only a matching terminal state may settle the wait, so none of
    // these may produce a toast (which is what a settle would do).
    const toasts: unknown[] = [];
    const off = onFutureEvent("toast", toast => void toasts.push(toast));
    render();
    await click("[data-compact]");

    emitAgentEvent({ threadId: "T2" });
    emitAgentEvent({ sessionId: "S9" });
    emitAgentEvent({ eventType: "message_delta" });
    emitAgentEvent({ payload: undefined });
    emitAgentEvent({ payload: { operation_id: "other" } });
    emitAgentEvent({ payload: {} });
    await flush();

    expect(h.client.compactThreadContext).toHaveBeenCalledTimes(1);
    expect(toasts).toEqual([]);
    off();
  });

  it("reports a failed compaction with the backend's reason", async () => {
    const toasts: { message: string }[] = [];
    const off = onFutureEvent("toast", toast => void toasts.push(toast));
    render();
    await click("[data-compact]");
    emitAgentEvent({ eventType: "compaction_failed", payload: { error: "context too small", operation_id: "op-1" } });
    await flush();

    expect(toasts).toHaveLength(1);
    expect(toasts[0]!.message).toContain("context too small");
    off();
  });

  it("names an unnamed compaction failure instead of printing 'undefined'", async () => {
    const toasts: { message: string }[] = [];
    const off = onFutureEvent("toast", toast => void toasts.push(toast));
    render();
    await click("[data-compact]");
    emitAgentEvent({ eventType: "compaction_failed", payload: { operation_id: "op-1" } });
    await flush();

    expect(toasts).toHaveLength(1);
    expect(toasts[0]!.message).toContain("The run failed. Please try again later.");
    off();
  });

  it.each([
    [true, "No conversation content was added after the last compaction."],
    [false, "There is no conversation context to compact."],
  ])("explains an unchanged compaction (already_compacted=%s)", async (alreadyCompacted, wording) => {
    const toasts: { message: string; tone?: string }[] = [];
    const off = onFutureEvent("toast", toast => void toasts.push(toast));
    render();
    await click("[data-compact]");
    emitAgentEvent({
      eventType: "compaction_unchanged",
      payload: { already_compacted: alreadyCompacted, operation_id: "op-1" },
    });
    await flush();

    expect(toasts).toEqual([{ message: wording, tone: "info" }]);
    off();
  });

  it("reports a compaction request the agent refused", async () => {
    // error-path: the invoke fails; the wait must settle rather than hang.
    const toasts: { message: string }[] = [];
    const off = onFutureEvent("toast", toast => void toasts.push(toast));
    h.client.compactThreadContext.mockRejectedValue(new Error("agent is busy"));
    render();
    await click("[data-compact]");
    await flush();

    expect(toasts).toHaveLength(1);
    expect(toasts[0]!.message).toContain("agent is busy");
    off();
  });

  it("times out rather than waiting forever for the agent's compaction result", async () => {
    // error-path: the backend can accept the request and never emit a terminal
    // event (a crashed worker). The wait is bounded so the composer unlocks.
    vi.useFakeTimers();
    try {
      const toasts: { message: string }[] = [];
      const off = onFutureEvent("toast", toast => void toasts.push(toast));
      render();
      await act(async () => {
        container.querySelector<HTMLButtonElement>("[data-compact]")!.click();
        await Promise.resolve();
        await vi.advanceTimersByTimeAsync(30 * 60 * 1000 + 1);
      });

      expect(toasts).toHaveLength(1);
      expect(toasts[0]!.message).toContain("Timed out waiting");
      off();
    }
    finally {
      vi.useRealTimers();
    }
  });

  it("stops waiting when the conversation is closed mid-compaction", async () => {
    // concurrency: unmounting cancels the wait, and a cancellation must not be
    // reported to the user as a failure.
    const toasts: unknown[] = [];
    const off = onFutureEvent("toast", toast => void toasts.push(toast));
    render();
    await act(async () => {
      container.querySelector<HTMLButtonElement>("[data-compact]")!.click();
      await Promise.resolve();
    });

    act(() => root.unmount());
    await flush();

    expect(toasts).toEqual([]);
    off();
    // Recreate the root so the shared `afterEach` unmount stays valid.
    root = createRoot(container);
  });
});

describe("agentThread agent-connection notice", () => {
  const cases: [string, AgentConnectionState, string, string][] = [
    ["agent unreachable", { kind: "agent_unavailable", status: "disconnected" }, "FutureOS background service is not running", "Retry"],
    ["a model catalog error", { error: "the catalog is corrupt", kind: "model_error", status: "disconnected" }, "the catalog is corrupt", "Retry"],
    ["an unclassified disconnect", { kind: "unknown", status: "disconnected" }, "Connection error", "Retry"],
    ["a login that is needed", { readiness: "needs_login", status: "connected" }, "No model configured", "Configure providers"],
    ["every model disabled", { readiness: "all_disabled", status: "connected" }, "All models disabled", "Model settings"],
    ["no models configured", { readiness: "no_models", status: "connected" }, "No models available", "Model settings"],
  ];

  it.each(cases)("shows the notice for %s", (_case, connection, title, label) => {
    render({ agentConnection: connection });
    expect(notice()).not.toBeNull();
    expect(notice()!.textContent).toContain(title);
    expect(noticeButtonLabel()).toBe(label);
  });

  it("stays silent for a healthy or still-checking connection", () => {
    render();
    expect(notice()).toBeNull();

    rerender({ agentConnection: { status: "checking" } });
    expect(notice()).toBeNull();
  });

  it("falls back to the generic detail when a disconnect carries no error text", () => {
    render({ agentConnection: { error: null, kind: "model_error", status: "disconnected" } });
    expect(notice()!.textContent).toContain("The background service is connected, but the model list couldn't be loaded");

    rerender({ agentConnection: { error: null, kind: "unknown", status: "disconnected" } });
    expect(notice()!.textContent).toContain("Please reopen FutureOS and try again.");
  });

  it("routes each notice's action to the right shell handler", async () => {
    async function clickNotice() {
      await act(async () => {
        notice()!.querySelector<HTMLButtonElement>("button")!.click();
        await Promise.resolve();
      });
    }

    render({ agentConnection: { kind: "agent_unavailable", status: "disconnected" } });
    await clickNotice();
    expect(props.onRetryAgentConnection).toHaveBeenCalledTimes(1);

    rerender({ agentConnection: { readiness: "needs_login", status: "connected" } });
    await clickNotice();
    expect(props.onOpenProviders).toHaveBeenCalledTimes(1);

    rerender({ agentConnection: { readiness: "all_disabled", status: "connected" } });
    await clickNotice();
    expect(props.onOpenModels).toHaveBeenCalledTimes(1);

    expect(props.onOpenAccount).not.toHaveBeenCalled();
  });
});
