import type { AgentMessage } from "@future-os/thread-projection";
// @vitest-environment jsdom
import type { Root } from "react-dom/client";
import type { AgentConnectionState } from "../../components/layout/AppShell";
import type { StoredThread } from "../../integrations/storage/threadStore";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { clearCommandHandlers } from "../keyboard/commands";
import { AgentThread } from "./AgentThread";

/**
 * What each `chrome` draws — the half of the Vibe panel's invariant that belongs
 * to the conversation view, and the reason the panel can promise "at most one
 * editable element" without policing focus:
 *
 * - `read` (a grid cell, and the panel's reading layer) renders **no composer at
 *   all**, so there is no editor to focus and nothing a dictation could land in;
 * - `input` renders exactly one, in the flow, with the caller's own target bar
 *   above and hint line below;
 * - `full` is unchanged: header, search, composer floating over the transcript.
 *
 * The panel's side of the invariant (only one cell renders `input` at a time) is
 * `VibeStage.test.tsx`.
 */
const h = vi.hoisted(() => ({
  threadState: { value: {} as Record<string, unknown> },
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
}));

vi.mock("./ThreadHeader", () => ({
  ThreadHeader: () => <div data-header="" />,
}));
vi.mock("./search/ThreadSearch", () => ({ ThreadSearch: () => <div data-search="" /> }));
vi.mock("./Composer", () => ({
  Composer: (props: {
    onEmptySubmit?: () => void;
    onSend?: (payload: { attachments: unknown[]; content: string }, resolve: () => void, reject: (error: unknown) => void) => void;
  }) => (
    <div data-composer="" data-empty-submit={String(typeof props.onEmptySubmit === "function")}>
      {/* The editor is the one thing that makes this composer a place to dictate
          into; the real component's contentEditable is what this stands for. */}
      <div contentEditable data-editor="" suppressContentEditableWarning />
      {/* What the real composer does with a send: hand the payload over with the two
          callbacks it awaits. The view's own behaviour is what reads them. */}
      {props.onSend
        ? (
            <button
              data-send=""
              onClick={() => void props.onSend!({ attachments: [], content: "a sentence" }, () => {}, () => {})}
              type="button"
            />
          )
        : null}
    </div>
  ),
}));
vi.mock("./MessageList", () => ({
  MessageList: (props: { messages: AgentMessage[]; spacing?: string }) => (
    <div data-count={props.messages.length} data-list="" data-spacing={props.spacing ?? "default"} />
  ),
}));
vi.mock("./ApprovalPrompt", () => ({
  ApprovalPrompt: () => <div data-approval="" />,
}));
vi.mock("../../components/ui/FloatingScrollbar", () => ({
  FloatingScrollbar: () => <div data-scrollbar="" />,
}));
vi.mock("./useAgentThreadState", () => ({ useAgentThreadState: () => h.threadState.value }));
vi.mock("./useMessagePaging", () => ({
  useMessagePaging: () => h.paging,
}));
vi.mock("../../lib/useFloatingScrollbar", () => ({
  useFloatingScrollbar: () => ({
    handleScroll: vi.fn(),
    handleThumbPointerDown: vi.fn(),
    scrollbar: null,
    scrollRef: { current: null },
    updateFloatingScrollbar: vi.fn(),
  }),
}));
vi.mock("./useComposerInset", () => ({
  useComposerInset: () => ({ composerHeight: 0, composerRef: { current: null } }),
}));
vi.mock("./useSkillRecommendation", () => ({
  useSkillRecommendation: () => ({ dismiss: vi.fn(), evaluate: vi.fn(async () => null), state: { recommendation: null } }),
}));
vi.mock("../../integrations/agent/agentStateCache", () => ({
  useCachedAgentState: () => null,
}));
vi.mock("../../integrations/storage/threadStore", () => ({ forkThread: vi.fn() }));
vi.mock("../../integrations/agent/agentClient", () => ({ compactThreadContext: vi.fn() }));
vi.mock("../skills/installRecommendedSkill", () => ({ installRecommendedSkill: vi.fn() }));

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

const thread = { agentSessionId: "S1", id: "T1", mode: "workspace" } as StoredThread;
const ready: AgentConnectionState = { readiness: "ready", status: "connected" };

function message(id: string, role: "user" | "assistant"): AgentMessage {
  return { content: id, id, role, status: "complete" } as AgentMessage;
}

let root: Root;
let container: HTMLDivElement;
let props: Parameters<typeof AgentThread>[0];

beforeEach(() => {
  clearCommandHandlers();
  h.paging.visibleMessages = [message("u1", "user"), message("a1", "assistant"), message("u2", "user"), message("a2", "assistant")];
  h.threadState.value = {
    handleAbort: vi.fn(async () => {}),
    handleSend: vi.fn(async () => {}),
    historyError: null,
    loadingIndicator: false,
    loadingThread: false,
    messages: h.paging.visibleMessages,
    renderWorkspace: { workspaceId: "W1", workspacePath: "/work" },
    retryHistory: vi.fn(async () => {}),
    sessionChanged: false,
  };
  props = {
    agentConnection: ready,
    approvalTier: "off",
    futureBalance: null,
    futureSessionStatus: "authenticated",
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
  container = document.body.appendChild(document.createElement("div"));
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

function render(over: Partial<Parameters<typeof AgentThread>[0]> = {}) {
  act(() => root.render(<AgentThread {...props} {...over} />));
}

const count = (selector: string) => container.querySelectorAll(selector).length;

describe("chrome: full", () => {
  it("draws the window's own chrome", () => {
    render();
    expect(count("[data-header]")).toBe(1);
    expect(count("[data-search]")).toBe(1);
    expect(count("[data-composer]")).toBe(1);
    expect(count("[data-list]")).toBe(1);
    // The whole history, not a tail.
    expect(container.querySelector("[data-list]")!.getAttribute("data-count")).toBe("4");
  });
});

describe("breathing room", () => {
  // Reported from use: a conversation in the panel's reading layer read as a solid column —
  // it started flush against the bar and ended flush against the hint line, and its turns
  // sat at the main window's (densely tuned) rhythm. The panel has more room and no composer
  // band under it, so it asks for more air; the main window is untouched apart from a bottom
  // inset it never had.
  it("gives the reading layer a roomier transcript than the main window", () => {
    render();
    expect(container.querySelector("[data-list]")!.getAttribute("data-spacing")).toBe("default");
    expect(container.querySelector("[data-chat-scroll]")!.className).toContain("pt-8");

    render({ chrome: "read" });
    expect(container.querySelector("[data-list]")!.getAttribute("data-spacing")).toBe("roomy");
    const scroll = container.querySelector("[data-chat-scroll]")!.className;
    // Top and bottom both: the column starts below the bar and stops before the hint line.
    expect(scroll).toContain("pt-10");
    expect(scroll).toContain("pb-16");
  });

  it("gives the input layer the same room, because it is the same transcript", () => {
    render({ chrome: "input", onLeaveInput: () => {} });
    expect(container.querySelector("[data-list]")!.getAttribute("data-spacing")).toBe("roomy");
  });
});

describe("chrome: read", () => {
  it("draws the transcript and no composer", () => {
    render({ chrome: "read" });
    expect(count("[data-composer]")).toBe(0);
    // The invariant, from this side: a cell has nowhere for a dictation to land.
    expect(count("[contenteditable=\"true\"]")).toBe(0);
    expect(container.querySelector("[data-list]")!.getAttribute("data-count")).toBe("4");
  });

  it("drops the window's chrome: no title bar, no find bar", () => {
    render({ chrome: "read" });
    expect(count("[data-header]")).toBe(0);
    expect(count("[data-search]")).toBe(0);
  });

  it("still shows an approval that needs an answer", () => {
    // The one thing a cell must be able to ask for, even without a composer.
    render({
      activeApproval: { id: "ap1" } as Parameters<typeof AgentThread>[0]["activeApproval"],
      chrome: "read",
    });
    expect(count("[data-approval]")).toBe(1);
  });
});

describe("chrome: input", () => {
  it("draws exactly one editor, in the flow, with the caller's bar and hint line", () => {
    render({
      chrome: "input",
      inputFooter: <div data-footer="" />,
      inputHeader: <div data-target="" />,
      onLeaveInput: () => {},
    });
    expect(count("[data-composer]")).toBe(1);
    expect(count("[contenteditable=\"true\"]")).toBe(1);
    expect(count("[data-target]")).toBe(1);
    expect(count("[data-footer]")).toBe(1);
    // Still no window chrome: the panel draws its own.
    expect(count("[data-header]")).toBe(0);
  });

  it("shows only the tail of the conversation", () => {
    // The two messages a speaker needs to remember what they were talking about.
    render({ chrome: "input", onLeaveInput: () => {} });
    expect(container.querySelector("[data-list]")!.getAttribute("data-count")).toBe("2");
  });

  it("hands the composer the empty-draft behaviour only when the layer can leave", () => {
    // An empty submit closes the layer when there is one; with nowhere to go it
    // must not be registered at all, or the key would swallow a press for nothing.
    render({ chrome: "input" });
    expect(container.querySelector("[data-composer]")!.getAttribute("data-empty-submit")).toBe("false");
    render({ chrome: "input", onLeaveInput: () => {} });
    expect(container.querySelector("[data-composer]")!.getAttribute("data-empty-submit")).toBe("true");
  });

  it("leaves the layer when a sentence is sent from it", () => {
    // The input layer exists to ask one question: once the draft is handed over (the
    // composer has already cleared it) the panel goes back to reading, so the answer
    // does not land behind the editor.
    const onLeaveInput = vi.fn();
    render({ chrome: "input", onLeaveInput });

    act(() => {
      container.querySelector<HTMLElement>("[data-send]")!.click();
    });

    expect(onLeaveInput).toHaveBeenCalledTimes(1);
  });

  it("stays put when a sentence is sent from the full chrome", () => {
    // The main window's composer sends into a conversation that stays on screen; there
    // is no layer to leave, and the callback is not even handed over.
    const onLeaveInput = vi.fn();
    render({ onLeaveInput });

    act(() => {
      container.querySelector<HTMLElement>("[data-send]")!.click();
    });

    expect(onLeaveInput).not.toHaveBeenCalled();
  });

  it("never claims an approval card of its own", () => {
    // Approvals are answered in the reading layer: a card appearing above the
    // editor mid-dictation moves the one thing the eye is on, and its two-key
    // confirmation does not fit there.
    render({
      activeApproval: { id: "ap1" } as Parameters<typeof AgentThread>[0]["activeApproval"],
      chrome: "input",
      onLeaveInput: () => {},
    });
    expect(count("[data-approval]")).toBe(0);
  });
});

describe("outside the panel", () => {
  it("keeps the default chrome when nothing says otherwise", () => {
    render();
    // `full` is the default: an existing caller never has to know the prop exists.
    expect(count("[data-header]")).toBe(1);
    expect(count("[data-composer]")).toBe(1);
  });
});
