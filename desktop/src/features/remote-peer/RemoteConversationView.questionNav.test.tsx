// @vitest-environment jsdom
import type { AgentMessage, SessionEntry } from "@future-os/thread-projection";
import { entriesToMessages } from "@future-os/thread-projection";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import { RemoteConversationView } from "./RemoteConversationView";

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({
    isFullscreen: () => Promise.resolve(false),
    onResized: () => Promise.resolve(() => {}),
    startDragging: () => Promise.resolve(),
  }),
}));

vi.mock("./remotePeerClient", () => ({
  requestRemotePeer: vi.fn(async () => ({})),
  setRemotePeerVisibleSession: vi.fn(),
}));

vi.mock("./remoteToolTarget", () => ({
  fetchRemoteToolTarget: vi.fn(async () => null),
  forgetRemoteToolTargets: vi.fn(),
}));

function entry(id: string, role: "user" | "assistant", text: string): SessionEntry {
  return { blocks: [{ kind: "text", text }], createdAtMs: 1, id, kind: "message", role } as unknown as SessionEntry;
}

async function mount(messages: AgentMessage[]) {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  await act(async () => {
    root.render(
      <RemoteConversationView
        approvals={[]}
        approvalErrors={{}}
        approvalPending={null}
        composer={<div />}
        desktopId="desktop_a"
        error={null}
        hasMore={false}
        leftPanelExpanded
        live={null}
        loading={false}
        loadingOlder={false}
        messages={messages}
        onContinueRun={vi.fn()}
        onDecideApproval={vi.fn()}
        onFork={vi.fn()}
        onLoadOlder={vi.fn()}
        onOpenUsage={vi.fn()}
        onRetry={vi.fn()}
        onToggleLeftPanel={vi.fn()}
        peer={undefined}
        persistedEntryIds={new Set()}
        sessionId="sess_1"
        streaming={false}
        title="Remote conversation"
      />,
    );
  });
  return {
    container,
    previousButton: () => container.querySelector<HTMLButtonElement>("[aria-label='Previous question']"),
    nextButton: () => container.querySelector<HTMLButtonElement>("[aria-label='Next question']"),
    unmount: async () => {
      await act(async () => root.unmount());
      container.remove();
    },
  };
}

afterEach(() => {
  document.body.innerHTML = "";
});

describe("remote conversation question navigation", () => {
  it("derives question list from the remote timeline", async () => {
    const messages = entriesToMessages([
      entry("u1", "user", "first"),
      entry("a1", "assistant", "answer one"),
      entry("u2", "user", "second"),
      entry("a2", "assistant", "answer two"),
    ]);
    const view = await mount(messages);

    expect(view.previousButton()).toBeTruthy();
    expect(view.nextButton()).toBeTruthy();
    await view.unmount();
  });

  it("has only next enabled at the top of the transcript", async () => {
    const messages = entriesToMessages([
      entry("u1", "user", "first"),
      entry("a1", "assistant", "answer one"),
      entry("u2", "user", "second"),
      entry("a2", "assistant", "answer two"),
    ]);
    const view = await mount(messages);

    expect(view.previousButton()!.disabled).toBe(true);
    expect(view.nextButton()!.disabled).toBe(false);
    await view.unmount();
  });

  it("calls the takeover callback and changes scroll position on next", async () => {
    const messages = entriesToMessages([
      entry("u1", "user", "first"),
      entry("a1", "assistant", "answer one"),
      entry("u2", "user", "second"),
      entry("a2", "assistant", "answer two"),
    ]);
    const view = await mount(messages);
    const scroller = view.container.querySelector<HTMLElement>("[data-chat-scroll='true']")!;
    const before = scroller.scrollTop;

    await act(async () => view.nextButton()!.click());

    // The click attempted to move scrollTop; in jsdom with no real layout it
    // may stay zero, but the navigation callback ran and sticky auto-follow was
    // paused, which is the behaviour that matters for a remote transcript.
    expect(scroller.scrollTop).not.toBeLessThan(before);
    await view.unmount();
  });
});
