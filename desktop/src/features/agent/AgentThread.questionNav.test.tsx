// @vitest-environment jsdom
import type { AgentMessage } from "@future-os/thread-projection";
import type { ReactNode } from "react";
import type { Root } from "react-dom/client";
import type { AgentConnectionState } from "../../components/layout/AppShell";
import type { StoredThread } from "../../integrations/storage/threadStore";
import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { AgentThread } from "./AgentThread";

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

const h = {
  threadState: { value: {} as Record<string, unknown> },
};

vi.mock("./ThreadHeader", () => ({
  ThreadHeader: (props: { action?: ReactNode }) => <div data-header="">{props.action ?? null}</div>,
}));
vi.mock("./search/ThreadSearch", () => ({ ThreadSearch: () => <div data-search="" /> }));
vi.mock("./Composer", () => ({
  Composer: (props: Record<string, unknown>) => (
    <div data-composer="" data-disabled={String(props.disabled)}>
      <button data-send="" onClick={() => (props.onSend as (p: unknown) => Promise<void>)?.({ content: "" })} type="button" />
    </div>
  ),
}));
vi.mock("./MessageList", () => ({
  MessageList: (props: { messages: AgentMessage[] }) => (
    <div data-list="">
      {props.messages.map(m => (
        <div key={m.id} data-message-id={m.id} data-role={m.role}>{m.content}</div>
      ))}
    </div>
  ),
}));
vi.mock("./ApprovalPrompt", () => ({ ApprovalPrompt: () => <div data-approval="" /> }));
vi.mock("../../components/ui/FloatingScrollbar", () => ({ FloatingScrollbar: () => <div data-scrollbar="" /> }));
vi.mock("./useAgentThreadState", () => ({ useAgentThreadState: () => h.threadState.value }));
vi.mock("./useMessagePaging", () => ({
  useMessagePaging: () => ({
    visibleMessages: h.threadState.value.messages,
    preserveViewport: vi.fn(),
    scrollRef: { current: null },
    scrollToLatest: vi.fn(),
    showJumpToLatest: false,
    showLoadOlderHint: false,
    coolingDown: false,
    handleScroll: vi.fn(),
    loadOlder: vi.fn(),
    prepareSearch: vi.fn(),
    revealSearchMatch: vi.fn(),
  }),
}));
vi.mock("../../lib/useFloatingScrollbar", () => ({
  useFloatingScrollbar: () => ({
    scrollRef: { current: null },
    scrollbar: null,
    updateFloatingScrollbar: vi.fn(),
    handleScroll: vi.fn(),
    handleThumbPointerDown: vi.fn(),
  }),
}));
vi.mock("./useComposerInset", () => ({ useComposerInset: () => ({ composerHeight: 0, composerRef: { current: null } }) }));
vi.mock("./useSkillRecommendation", () => ({ useSkillRecommendation: () => ({ state: { recommendation: null }, evaluate: vi.fn() }) }));
vi.mock("../../integrations/agent/agentStateCache", () => ({ useCachedAgentState: () => null }));
vi.mock("./runtime/useThreadCompaction", () => ({ useThreadCompaction: () => vi.fn() }));
vi.mock("../skills/installRecommendedSkill", () => ({ installRecommendedSkill: vi.fn() }));

const thread = { id: "T1", mode: "workspace" } as StoredThread;
const ready: AgentConnectionState = { readiness: "ready", status: "connected" };

function user(id: string): AgentMessage {
  return { content: `prompt ${id}`, id, role: "user", status: "complete", createdAt: "2026-01-01T00:00:00Z" } as AgentMessage;
}
function assistant(id: string): AgentMessage {
  return { content: `answer ${id}`, id, role: "assistant", status: "complete", createdAt: "2026-01-01T00:00:00Z" } as AgentMessage;
}

let root: Root;
let container: HTMLDivElement;

beforeEach(() => {
  h.threadState.value = {
    handleAbort: vi.fn(),
    handleSend: vi.fn(async () => {}),
    historyError: null,
    loadingIndicator: false,
    loadingThread: false,
    messages: [user("u1"), assistant("a1"), user("u2"), assistant("a2")],
    renderWorkspace: { workspaceId: "W1", workspacePath: "/work" },
    retryHistory: vi.fn(),
    sessionChanged: false,
  };

  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

const props = {
  agentConnection: ready,
  approvalTier: "manual" as const,
  futureBalance: 10,
  futureSessionStatus: "authenticated",
  headerAction: null,
  leftPanelExpanded: true,
  loadingStore: false,
  modelId: "m1",
  modelOptions: [],
  onApprovalDecision: vi.fn(),
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

/**
 * Give the mocked transcript a real scroll geometry.
 *
 * The hook decides what the buttons offer from the rows' own layout, and jsdom
 * reports every element at zero — so without this the conversation would look
 * like one that fits its viewport with nothing to jump to (which is what the
 * second case here is about). Rows are stacked `rowHeight` apart in content
 * space; the container scrolls from its top.
 */
function layoutTranscript(rowHeight = 120, viewportHeight = 400) {
  const scroller = container.querySelector<HTMLElement>("[data-chat-scroll]");
  if (!scroller)
    throw new Error("the transcript scroller is missing");
  const rows = [...container.querySelectorAll<HTMLElement>("[data-message-id]")];
  const rect = (top: number, height: number): DOMRect => ({
    bottom: top + height,
    height,
    left: 0,
    right: 600,
    toJSON: () => ({}),
    top,
    width: 600,
    x: 0,
    y: top,
  });
  Object.defineProperty(scroller, "scrollHeight", { configurable: true, value: rows.length * rowHeight + viewportHeight });
  Object.defineProperty(scroller, "clientHeight", { configurable: true, value: viewportHeight });
  vi.spyOn(scroller, "getBoundingClientRect").mockReturnValue(rect(0, viewportHeight));
  rows.forEach((row, index) => {
    vi.spyOn(row, "getBoundingClientRect").mockImplementation(() =>
      rect(index * rowHeight - scroller.scrollTop, rowHeight));
  });
  // The offer is published from a scroll (the hook listens for one so an
  // enabled state cannot lag behind the viewport). jsdom never fires one by
  // itself, so the layout becoming real is announced here.
  act(() => scroller.dispatchEvent(new Event("scroll")));
}

describe("agentThread question navigation", () => {
  it("renders question navigation controls in the conversation's corner", () => {
    act(() => root.render(createElement(AgentThread, props)));
    const previous = container.querySelector<HTMLButtonElement>("[aria-label='Previous question']");
    const next = container.querySelector<HTMLButtonElement>("[aria-label='Next passage']");
    expect(previous).not.toBeNull();
    expect(next).not.toBeNull();

    // At the top of two turns: the first row is the reader's own, so ↑ has
    // nowhere to go, while ↓ has the first turn's reply below it.
    act(() => layoutTranscript());
    expect(previous?.disabled).toBe(true);
    expect(next?.disabled).toBe(false);
  });

  it("disables both directions for a single unanswered question", () => {
    h.threadState.value = {
      ...h.threadState.value,
      messages: [user("u1")],
    };
    act(() => root.render(createElement(AgentThread, props)));
    act(() => layoutTranscript());
    const next = container.querySelector<HTMLButtonElement>("[aria-label='Next passage']");
    const previous = container.querySelector<HTMLButtonElement>("[aria-label='Previous question']");
    expect(next?.disabled).toBe(true);
    expect(previous?.disabled).toBe(true);
  });
});
