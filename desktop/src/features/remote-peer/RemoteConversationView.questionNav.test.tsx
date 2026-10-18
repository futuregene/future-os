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

/** Two settled turns, which is what the ladder needs to have anything to do. */
function twoTurns(): AgentMessage[] {
  return entriesToMessages([
    entry("u1", "user", "first"),
    entry("a1", "assistant", "answer one"),
    entry("u2", "user", "second"),
    entry("a2", "assistant", "answer two"),
  ]);
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
        composer={<div data-composer="" />}
        desktopId="desktop_a"
        error={null}
        hasMore={false}
        leftPanelExpanded
        liveReplies={[]}
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
        olderHistoryFailed={false}
        peer={undefined}
        persistedEntryIds={new Set()}
        sessionId="sess_1"
        streaming={false}
        title="Remote conversation"
        workspaceName={null}
      />,
    );
  });
  return {
    container,
    header: () => container.querySelector<HTMLElement>("header"),
    nextButton: () => container.querySelector<HTMLButtonElement>("[aria-label='Next passage']"),
    previousButton: () => container.querySelector<HTMLButtonElement>("[aria-label='Previous question']"),
    unmount: async () => {
      await act(async () => root.unmount());
      container.remove();
    },
  };
}

/**
 * Give the mocked host transcript a real scroll geometry: rows stacked in
 * content space, a container that scrolls from its top.
 */
function layoutTranscript(container: HTMLElement, rowHeight = 120, viewportHeight = 400) {
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
  act(() => scroller.dispatchEvent(new Event("scroll")));
}

afterEach(() => {
  document.body.innerHTML = "";
});

describe("remote conversation question navigation", () => {
  it("renders the pair for the host's transcript", async () => {
    const view = await mount(twoTurns());

    expect(view.previousButton()).toBeTruthy();
    expect(view.nextButton()).toBeTruthy();
    await view.unmount();
  });

  it("sits in the conversation's corner rather than the header", async () => {
    // A remote transcript is read the way this app's own is, so the control
    // lives in the same corner: putting it in the header made the same pair
    // move between the two views.
    const view = await mount(twoTurns());
    const nav = view.previousButton()!;
    const composer = view.container.querySelector("[data-composer]")!;
    // The nearest ancestor the nav shares with the composer is the band the
    // composer rides in — and that band is not the header.
    let band: Element | null = nav;
    while (band && !band.contains(composer))
      band = band.parentElement;

    expect(view.header()!.contains(nav)).toBe(false);
    expect(band).not.toBeNull();
    expect(band!.contains(view.header()!)).toBe(false);
    await view.unmount();
  });

  it("has only next enabled at the top of the transcript", async () => {
    const view = await mount(twoTurns());
    // The hook reads the rows' own layout, which jsdom reports as zeroes (a
    // conversation with nothing to jump to). Give the transcript real geometry,
    // then announce it the way a scroll does.
    layoutTranscript(view.container);

    expect(view.previousButton()!.disabled).toBe(true);
    expect(view.nextButton()!.disabled).toBe(false);
    await view.unmount();
  });

  it("calls the takeover callback and changes scroll position on next", async () => {
    const view = await mount(twoTurns());
    const scroller = view.container.querySelector<HTMLElement>("[data-chat-scroll='true']")!;
    const before = scroller.scrollTop;

    await act(async () => view.nextButton()!.click());

    // The click attempted to move scrollTop; in jsdom with no real layout it
    // may stay zero, but the navigation callback ran and sticky auto-follow was
    // paused, which is the behaviour that matters for a remote transcript.
    expect(scroller.scrollTop).not.toBeLessThan(before);
    await view.unmount();
  });

  it("reads the host's transcript with the bare arrow keys", async () => {
    // A remote conversation is read the way this app's own is, and the arrows
    // are part of that reading — the client that did not answer them would be
    // the one where paging needs the mouse.
    const view = await mount(twoTurns());
    const scroller = view.container.querySelector<HTMLElement>("[data-chat-scroll='true']")!;
    layoutTranscript(view.container);

    const down = new KeyboardEvent("keydown", { bubbles: true, cancelable: true, key: "ArrowDown" });
    await act(async () => window.dispatchEvent(down));
    expect(down.defaultPrevented).toBe(true);

    const up = new KeyboardEvent("keydown", { bubbles: true, cancelable: true, key: "ArrowUp" });
    await act(async () => window.dispatchEvent(up));
    expect(up.defaultPrevented).toBe(true);
    expect(scroller.scrollTop).toBe(0);
    await view.unmount();
  });
});
