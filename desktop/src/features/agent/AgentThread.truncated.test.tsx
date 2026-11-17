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
vi.mock("./Composer", () => ({ Composer: () => <div data-composer="" /> }));
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
vi.mock("./useSkillRecommendation", () => ({
  useSkillRecommendation: () => ({ state: { recommendation: null }, evaluate: vi.fn() }),
}));
vi.mock("../../integrations/agent/agentStateCache", () => ({ useCachedAgentState: () => null }));
vi.mock("./runtime/useThreadCompaction", () => ({ useThreadCompaction: () => vi.fn() }));
vi.mock("../skills/installRecommendedSkill", () => ({ installRecommendedSkill: vi.fn() }));

const thread = { id: "T1", mode: "workspace" } as StoredThread;
const ready: AgentConnectionState = { readiness: "ready", status: "connected" };

function answer(id: string): AgentMessage {
  return { content: `answer ${id}`, id, role: "assistant", status: "complete", createdAt: "2026-01-01T00:00:00Z" } as AgentMessage;
}

let root: Root;
let container: HTMLDivElement;

beforeEach(() => {
  h.threadState.value = {
    handleAbort: vi.fn(),
    handleSend: vi.fn(async () => {}),
    hasOlderHistory: false,
    historyError: null,
    historyTruncated: false,
    loadingIndicator: false,
    loadingThread: false,
    // The shape a mid-turn page projects to: a reply with no question in front
    // of it. The page is not empty — it is a fragment — and the reader has to be
    // told that, because nothing in the transcript says it.
    messages: [answer("a1"), answer("a2")],
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

function notice() {
  return container.querySelector<HTMLElement>("[role='note']");
}

describe("agentThread mid-turn pages", () => {
  it("says a page that opens mid-turn starts mid-turn", () => {
    h.threadState.value.historyTruncated = true;
    act(() => root.render(createElement(AgentThread, props)));
    expect(notice()?.textContent).toBe(
      "This turn is too long to show from its question. Load earlier messages to read it from the start.",
    );
  });

  it("says nothing when the page starts at its question", () => {
    act(() => root.render(createElement(AgentThread, props)));
    expect(notice()).toBeNull();
  });
});
