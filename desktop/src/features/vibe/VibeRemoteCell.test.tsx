// @vitest-environment jsdom
import type { RemoteCatalog, RemoteSessionRow } from "../remote-peer/mergeConversations";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { VibeRemoteCell } from "./VibeRemoteCell";

/**
 * One host's conversation as a panel cell.
 *
 * The cell is an adapter, and this file is about the adapter alone: which host and
 * session it asks the timeline for, and what each of the view's five actions means on
 * this side. `RemoteConversationView` is stubbed **by the actions it offers** — the
 * only thing the cell is entitled to say about it — so what is asserted here is the
 * mapping, not the view's own rendering (`RemoteConversationView.test.tsx` holds that).
 *
 * The cell deliberately has no composer: the panel's one-editable-element rule is what
 * keeps dictation on this machine, and that is asserted by the stub receiving `null`.
 */
const mocks = vi.hoisted(() => ({
  timeline: {
    error: null as string | null,
    hasMore: false,
    liveReplies: new Map<string, string>(),
    loading: false,
    loadingOlder: false,
    loadOlder: vi.fn(async () => {}),
    messages: [] as unknown[],
    olderHistoryFailed: false,
    persistedEntryIds: new Set<string>(),
    refresh: vi.fn(async () => {}),
    streaming: false,
  },
  approvals: { approvals: [] as unknown[], decide: vi.fn(async () => {}), errors: {}, pending: false },
  continueRun: vi.fn(async () => {}),
  view: { props: null as null | Record<string, unknown> },
}));

vi.mock("../remote-peer/useRemoteTimeline", () => ({ useRemoteTimeline: () => mocks.timeline }));
vi.mock("../remote-peer/useRemoteApprovals", () => ({ useRemoteApprovals: () => mocks.approvals }));
vi.mock("../remote-peer/remotePeerClient", () => ({
  continueRemoteRun: (...args: unknown[]) => mocks.continueRun(...(args as [])),
}));
vi.mock("../remote-peer/RemoteConversationView", () => ({
  RemoteConversationView: (props: Record<string, unknown>) => {
    mocks.view.props = props;
    return (
      <div data-remote-view="">
        <button data-action="continue" onClick={() => (props.onContinueRun as (id: string) => void)("run_9")} type="button" />
        <button
          data-action="decide"
          onClick={() => (props.onDecideApproval as (a: unknown, d: string) => void)({ id: "ap_1" }, "approved")}
          type="button"
        />
        <button data-action="fork" onClick={() => (props.onFork as () => void)()} type="button" />
        <button data-action="usage" onClick={() => (props.onOpenUsage as () => void)()} type="button" />
        <button data-action="retry" onClick={() => (props.onRetry as () => void)()} type="button" />
        <button data-action="older" onClick={() => void (props.onLoadOlder as () => Promise<void>)()} type="button" />
      </div>
    );
  },
}));

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

const catalog = { desktopId: "desk_1", sessions: [] } as unknown as RemoteCatalog;
const session = { sessionId: "sess_1", title: "Remote", mode: "chat" } as unknown as RemoteSessionRow;

let container: HTMLDivElement;
let root: ReturnType<typeof createRoot>;

beforeEach(() => {
  container = document.body.appendChild(document.createElement("div"));
  root = createRoot(container);
  mocks.continueRun.mockClear();
  mocks.approvals.decide.mockClear();
  mocks.timeline.refresh.mockClear();
  mocks.timeline.loadOlder.mockClear();
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

function mount(onOpenInShell = vi.fn()) {
  act(() => root.render(<VibeRemoteCell catalog={catalog} onOpenInShell={onOpenInShell} peer={undefined} session={session} />));
  return onOpenInShell;
}

function press(action: string): void {
  act(() => container.querySelector<HTMLElement>(`[data-action="${action}"]`)!.click());
}

describe("the remote cell's five actions", () => {
  it("continues a run on the host that owns the conversation", () => {
    mount();
    press("continue");
    // The cell answers "which host, which session" once, and every action goes through it.
    expect(mocks.continueRun).toHaveBeenCalledWith("desk_1", "sess_1", "run_9");
  });

  it("answers an approval through that conversation's own queue", () => {
    mount();
    press("decide");
    expect(mocks.approvals.decide).toHaveBeenCalledWith({ id: "ap_1" }, "approved");
  });

  it("opens the conversation in the shell for both fork and usage", () => {
    // Two different buttons, one meaning here: this machine has no place to fork or to
    // account for a host's run, so both hand the conversation back to the shell.
    const onOpenInShell = mount();
    press("fork");
    press("usage");
    expect(onOpenInShell).toHaveBeenCalledTimes(2);
  });

  it("re-reads the timeline on retry, and asks for an older page on demand", () => {
    mount();
    press("retry");
    expect(mocks.timeline.refresh).toHaveBeenCalledTimes(1);
    press("older");
    expect(mocks.timeline.loadOlder).toHaveBeenCalledTimes(1);
  });

  it("draws no composer, and tells the view which host it is showing", () => {
    mount();
    const props = mocks.view.props!;
    expect(props.chrome).toBe("read");
    expect(props.composer).toBeNull();
    expect(props.desktopId).toBe("desk_1");
  });
});
