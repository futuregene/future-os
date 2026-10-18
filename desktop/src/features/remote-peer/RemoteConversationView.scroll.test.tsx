// @vitest-environment jsdom
import type { AgentMessage, SessionEntry } from "@future-os/thread-projection";
import type { RemoteLiveReply } from "./remoteRunStreams";
import { entriesToMessages } from "@future-os/thread-projection";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { RemoteConversationView } from "./RemoteConversationView";

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;
vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({
    isFullscreen: () => Promise.resolve(false),
    onResized: () => Promise.resolve(() => {}),
    startDragging: () => Promise.resolve(),
  }),
}));
vi.mock("./remoteToolTarget", () => ({
  fetchRemoteToolTarget: vi.fn(async () => null),
  forgetRemoteToolTargets: vi.fn(),
}));

function entry(id: string, role: "user" | "assistant", text: string): SessionEntry {
  return { blocks: [{ kind: "text", text }], createdAtMs: 1, id, kind: "message", role } as unknown as SessionEntry;
}

async function mount(props: {
  hasMore?: boolean;
  /** The reply the host is writing, if the test is about streaming. */
  liveReplies?: RemoteLiveReply[];
  loadingOlder?: boolean;
  messages: AgentMessage[];
  olderHistoryFailed?: boolean;
  onFork?: (sourceEntryId: string, forkable: boolean) => void;
  onLoadOlder?: () => void | Promise<unknown>;
  persisted?: string[];
}) {
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
        hasMore={props.hasMore ?? false}
        leftPanelExpanded
        liveReplies={props.liveReplies ?? []}
        loading={false}
        loadingOlder={props.loadingOlder ?? false}
        messages={props.messages}
        onContinueRun={vi.fn()}
        onDecideApproval={vi.fn()}
        onFork={props.onFork ?? vi.fn()}
        onLoadOlder={props.onLoadOlder ?? vi.fn()}
        onOpenUsage={vi.fn()}
        onRetry={vi.fn()}
        onToggleLeftPanel={vi.fn()}
        olderHistoryFailed={props.olderHistoryFailed ?? false}
        peer={undefined}
        persistedEntryIds={new Set(props.persisted ?? [])}
        sessionId="sess_1"
        streaming={false}
        title="A conversation"
        workspaceName={null}
      />,
    );
  });
  // The transcript viewport, addressed the way the app addresses it: the
  // `data-chat-scroll` hook is what the composer-inset and sticky-scroll code
  // key off, and it is the one contract the local and remote transcripts share.
  // (This used to match the overflow class, which is a layout detail of
  // whichever frame the transcript happens to sit in.)
  const scroller = () => container.querySelector<HTMLElement>("[data-chat-scroll='true']")!;
  return {
    container,
    fork: () => container.querySelector<HTMLButtonElement>("button[title='Fork']")!,
    scroll: async (from: number, to: number) => {
      await act(async () => {
        const element = scroller();
        element.scrollTop = from;
        element.dispatchEvent(new Event("scroll", { bubbles: true }));
        element.scrollTop = to;
        element.dispatchEvent(new Event("scroll", { bubbles: true }));
      });
    },
    unmount: async () => {
      await act(async () => root.unmount());
      container.remove();
    },
  };
}

/**
 * Scrolling up to the top asks for older history, without a button to press.
 *
 * The local conversation has always done this. A reader who scrolls up and simply
 * stops should get history, not a control they have to notice — and the trigger
 * is the *upward* move, so arriving at the top some other way (a short transcript,
 * a re-read) does not start a chain of loads.
 */
it("asks for older history when the reader scrolls up to the top", async () => {
  const onLoadOlder = vi.fn();
  const messages = entriesToMessages([entry("u1", "user", "hello"), entry("a1", "assistant", "hi")]);
  const view = await mount({ hasMore: true, messages, onLoadOlder });

  await view.scroll(300, 10);
  expect(onLoadOlder).toHaveBeenCalledTimes(1);

  // Scrolling down does not ask, and neither does sitting at the top.
  await view.scroll(10, 400);
  await view.scroll(400, 420);
  expect(onLoadOlder).toHaveBeenCalledTimes(1);
  await view.unmount();
});

/** With nothing older to load, the top of the transcript is just the top. */
it("asks for nothing when there is no more history", async () => {
  const onLoadOlder = vi.fn();
  const messages = entriesToMessages([entry("u1", "user", "hello")]);
  const view = await mount({ hasMore: false, messages, onLoadOlder });

  await view.scroll(300, 10);
  expect(onLoadOlder).not.toHaveBeenCalled();
  await view.unmount();
});

/** The pill the band over the transcript's top edge draws, if it is up. */
function olderPill(view: { container: HTMLElement }) {
  return view.container.querySelector<HTMLButtonElement>("button[aria-label='Load older']");
}

/**
 * The band over the transcript's top edge speaks only while it has something to
 * say.
 *
 * It used to be up whenever the host had more history — which is every remote
 * conversation that is not its first page — so the pill sat over the first line
 * of the transcript for as long as the window was open, covering what it was
 * next to. What is left is the feedback a request needs: a page on its way, or a
 * page that did not come.
 */
it("leaves the top of the transcript clear while the host's history is simply farther up", async () => {
  const messages = entriesToMessages([entry("u1", "user", "hello"), entry("a1", "assistant", "hi")]);
  const view = await mount({ hasMore: true, messages });

  expect(olderPill(view)).toBeNull();
  await view.unmount();
});

it("says a page is on its way, and is not pressable while it is", async () => {
  const messages = entriesToMessages([entry("u1", "user", "hello")]);
  const view = await mount({ hasMore: true, loadingOlder: true, messages });

  const pill = olderPill(view)!;
  expect(pill.textContent).toContain("Loading…");
  expect(pill.disabled).toBe(true);
  expect(pill.getAttribute("aria-busy")).toBe("true");
  await view.unmount();
});

it("offers the press again when the page did not come", async () => {
  const messages = entriesToMessages([entry("u1", "user", "hello")]);
  const onLoadOlder = vi.fn();
  const view = await mount({ hasMore: true, messages, olderHistoryFailed: true, onLoadOlder });

  const pill = olderPill(view)!;
  expect(pill.textContent).toContain("retry");
  expect(pill.disabled).toBe(false);
  expect(pill.getAttribute("aria-busy")).toBe("false");

  await act(async () => pill.click());
  expect(onLoadOlder).toHaveBeenCalledTimes(1);
  await view.unmount();
});

it("does not offer a retry for a page there is none of", async () => {
  const messages = entriesToMessages([entry("u1", "user", "hello")]);
  const view = await mount({ hasMore: false, messages, olderHistoryFailed: true });

  expect(olderPill(view)).toBeNull();
  await view.unmount();
});

/**
 * A fork branches at the *user* turn that produced the reply.
 *
 * A reply is not a stored turn on the host; the host forks at the user entry
 * behind it, and only accepts an id from its own store.
 */
it("forks at the preceding prompt, and says whether it is persisted", async () => {
  const onFork = vi.fn();
  const messages = entriesToMessages([
    entry("u1", "user", "do it"),
    entry("a1", "assistant", "done"),
  ]);
  const view = await mount({ messages, onFork, persisted: ["u1"] });

  await act(async () => view.fork().click());
  expect(onFork).toHaveBeenCalledWith("u1", true);
  await view.unmount();
});

/** A turn that arrived as a live push is reported, not sent as a stored id. */
it("reports a reply whose turn is not persisted on the host", async () => {
  const onFork = vi.fn();
  const messages = entriesToMessages([
    entry("u1", "user", "do it"),
    entry("a1", "assistant", "done"),
  ]);
  const view = await mount({ messages, onFork, persisted: [] });

  await act(async () => view.fork().click());
  expect(onFork).toHaveBeenCalledWith("u1", false);
  await view.unmount();
});
