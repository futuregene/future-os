// @vitest-environment jsdom
import type { SessionEntry } from "@future-os/thread-projection";
import type { RemotePeer } from "./remotePeerClient";
import { entriesToMessages } from "@future-os/thread-projection";
import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { ChoiceCardsProvider } from "../markdown/ChoiceCardsContext";
import { RemoteConversationView } from "./RemoteConversationView";

/**
 * A remote transcript draws a card only while *that machine* allows it.
 *
 * The value reaches the view as a prop (`useRemoteChoiceCards` decides it); what
 * this file pins is what the transcript does with it: off means no card **and**
 * no fenced source — the block leaves no trace — and leaving it out means this
 * machine's own switch, which is the rule the markdown renderer applies
 * everywhere else.
 */

vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({
    isFullscreen: () => Promise.resolve(false),
    onResized: () => Promise.resolve(() => {}),
    startDragging: () => Promise.resolve(),
  }),
}));
vi.mock("@tauri-apps/api/event", () => ({ listen: async () => () => {} }));
vi.mock("../../integrations/tauri/invoke", () => ({ invokeCommand: async () => ({}) }));

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

/** A reply that closes with a card, exactly as the agent is taught to write one. */
const REPLY = [
  "今天的检查都过了。",
  "",
  "```futureos-choice",
  "title: 接下来做哪一件？",
  "option: forest | 重画森林图",
  "option: flow | 补一张实验流程图",
  "```",
].join("\n");

function entry(id: string, role: "assistant" | "user", text: string): SessionEntry {
  return {
    id,
    kind: "message",
    role,
    createdAtMs: 1_000,
    blocks: [{ kind: "text", text }],
  };
}

const PEER: RemotePeer = {
  agentAvailable: true,
  bridgeInstanceId: "bridge_1",
  connected: true,
  stopped: false,
  desktopId: "desktop_a",
  error: null,
  features: [],
  icon: "laptop",
  name: "Studio iMac",
  pairId: "pair_1",
};

async function mount({ choiceCards, local = true, onChoiceDismiss, onChoiceSelect }: {
  choiceCards?: boolean;
  local?: boolean;
  onChoiceDismiss?: () => void;
  onChoiceSelect?: (question: unknown, optionId: string) => void;
}) {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  await act(async () => {
    root.render(
      createElement(
        ChoiceCardsProvider,
        { value: local },
        createElement(RemoteConversationView, {
          approvals: [],
          approvalErrors: {},
          approvalPending: null,
          ...(choiceCards === undefined ? {} : { choiceCards }),
          composer: createElement("div", { "data-testid": "composer" }),
          desktopId: "desktop_a",
          error: null,
          hasMore: false,
          inFlightRunId: null,
          leftPanelExpanded: true,
          liveReplies: [],
          loading: false,
          loadingOlder: false,
          messages: entriesToMessages([entry("u1", "user", "先看看今天的结果"), entry("e1", "assistant", REPLY)]),
          onContinueRun: vi.fn(),
          onChoiceDismiss,
          onChoiceSelect,
          onDecideApproval: vi.fn(),
          onFork: vi.fn(),
          onLoadOlder: () => {},
          onOpenUsage: vi.fn(),
          onRetry: () => {},
          onToggleLeftPanel: () => {},
          olderHistoryFailed: false,
          peer: PEER,
          persistedEntryIds: new Set(["u1", "e1"]),
          sessionId: "sess_1",
          streaming: false,
          title: "Remote conversation",
          workspaceName: null,
        }),
      ),
    );
  });
  return {
    container,
    cards: () => container.querySelectorAll("[data-choice-card]"),
    text: () => container.textContent ?? "",
    unmount: async () => {
      await act(async () => root.unmount());
      container.remove();
    },
  };
}

it("draws neither the card nor its source while that machine has the switch off", async () => {
  // This machine's own switch is on: the host's off still wins.
  const view = await mount({ choiceCards: false, local: true });
  try {
    expect(view.cards()).toHaveLength(0);
    expect(view.text()).not.toContain("futureos-choice");
    expect(view.text()).not.toContain("重画森林图");
    // The prose around it is untouched — only the block goes.
    expect(view.text()).toContain("今天的检查都过了。");
  }
  finally {
    await view.unmount();
  }
});

it("draws the card, pickable, while that machine has the switch on", async () => {
  // And this machine's off does not take it away: the host writes the cards.
  const view = await mount({ choiceCards: true, local: false });
  try {
    expect(view.cards().length).toBeGreaterThan(0);
    expect([...view.container.querySelectorAll<HTMLElement>("[data-choice-option]")]
      .map(row => row.dataset.choiceOption)).toEqual(["forest", "flow"]);
    expect(view.text()).toContain("接下来做哪一件？");
  }
  finally {
    await view.unmount();
  }
});

it("falls back to this machine's switch when the caller passes no answer", async () => {
  const on = await mount({ local: true });
  try {
    expect(on.cards().length).toBeGreaterThan(0);
  }
  finally {
    await on.unmount();
  }
  const off = await mount({ local: false });
  try {
    expect(off.cards()).toHaveLength(0);
    expect(off.text()).not.toContain("futureos-choice");
  }
  finally {
    await off.unmount();
  }
});

/**
 * "都不做" on a *remote* conversation (§5.9.4): the card is drawn by that machine,
 * and so is its mark — the reminder lives in its own rails and lists — so
 * declining goes back to that machine as a command, while the draft on this side
 * keeps nothing. This view only routes the click; the command, its address and
 * the toast are the caller's (`AppShell`).
 */
it("routes a decline back to the machine that drew the card", async () => {
  const onChoiceDismiss = vi.fn();
  const view = await mount({ choiceCards: true, onChoiceDismiss, onChoiceSelect: vi.fn() });
  try {
    const button = view.container.querySelector<HTMLElement>("[data-choice-dismiss]");
    expect(button).toBeTruthy();
    // The row sits outside the option group: it is not a third choice.
    expect([...view.container.querySelectorAll<HTMLElement>("[data-choice-option]")]).toHaveLength(2);
    act(() => button!.click());
    expect(onChoiceDismiss).toHaveBeenCalledTimes(1);
  }
  finally {
    await view.unmount();
  }
});

it("draws no way out when this side has nothing to clear the mark with", async () => {
  // A view mounted without a dismiss handler is a preview: it must not offer a
  // button whose click would go nowhere.
  const view = await mount({ choiceCards: true, onChoiceSelect: vi.fn() });
  try {
    expect(view.cards().length).toBeGreaterThan(0);
    expect(view.container.querySelector("[data-choice-dismiss]")).toBeNull();
  }
  finally {
    await view.unmount();
  }
});
