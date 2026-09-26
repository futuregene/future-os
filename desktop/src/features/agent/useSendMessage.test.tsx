// @vitest-environment jsdom
import type { MutableRefObject } from "react";
import type { StoredRun, StoredThread } from "../../integrations/storage/threadStore";
import type { ComposerSendPayload } from "./Composer";
import { act, useRef } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { onFutureEvent } from "../../lib/futureEvents";
import { renderHook } from "../../test/renderHook";
import { runSendPipeline } from "./sendPipeline";
import { useSendMessage } from "./useSendMessage";

vi.mock("./sendPipeline", () => ({ runSendPipeline: vi.fn() }));

const pipeline = vi.mocked(runSendPipeline);

const thread = { id: "thread-1", workspaceId: "w1", agentSessionId: "s1", title: "t" } as StoredThread;
const run = { id: "run-1", threadId: "thread-1", status: "running" } as StoredRun;
const payload: ComposerSendPayload = { attachments: [], content: "hello" };

interface Calls {
  messages: unknown[][];
  recentRuns: StoredRun[];
  activity: number;
  refreshes: unknown[][];
}

function setup(options: { thread?: StoredThread | null; activeRunId?: string | null } = {}) {
  const calls: Calls = { messages: [], recentRuns: [], activity: 0, refreshes: [] };
  let sending!: MutableRefObject<boolean>;
  const harness = renderHook(() => {
    const sendingRef = useRef(false);
    sending = sendingRef;
    return useSendMessage({
      thread: options.thread === undefined ? thread : options.thread,
      modelId: "model-1",
      thinkingLevel: "high",
      activeRunId: options.activeRunId ?? null,
      sendingRef,
      setMessages: (update) => {
        calls.messages.push([update]);
      },
      setRecentRun: r => void calls.recentRuns.push(r),
      refreshRecentRun: async (threadId, workspaceId) => {
        calls.refreshes.push([threadId, workspaceId]);
      },
      onThreadActivity: () => void (calls.activity += 1),
    });
  });
  return {
    harness,
    calls,
    sendingRef: () => sending,
    /**
     * Start a send. Nothing is wrapped in `act`: the hook only mutates refs, so
     * a scope would have to be left open across a held pipeline.
     */
    send: (onAccepted?: () => void) => harness.current.handleSend(payload, onAccepted),
  };
}

/** Collect every toast this test emits. */
function collectToasts() {
  const toasts: { message: string; tone?: string }[] = [];
  const off = onFutureEvent("toast", toast => void toasts.push(toast));
  return { toasts, off };
}

/** A pipeline run that stays in flight until the returned resolver is called. */
function heldPipeline() {
  let release: () => void = () => {};
  pipeline.mockImplementation(() => new Promise<void>((resolve) => {
    release = resolve;
  }));
  return () => release();
}

beforeEach(() => {
  pipeline.mockReset();
  pipeline.mockResolvedValue(undefined);
});

describe("useSendMessage", () => {
  it("rejects a send with no active conversation, but only when the caller waits", async () => {
    const { harness, calls, sendingRef, send } = setup({ thread: null });

    // The composer path passes `onAccepted`, so the failure must surface.
    await expect(send(() => {})).rejects.toThrow("No active conversation");
    // boundary: the fire-and-forget path (recover-run) must not throw.
    await expect(send()).resolves.toBeUndefined();

    expect(pipeline).not.toHaveBeenCalled();
    expect(calls.messages).toEqual([]);
    expect(sendingRef().current).toBe(false);
    harness.unmount();
  });

  it("refuses a second prompt while a local send owns the view", async () => {
    const { harness, sendingRef, send } = setup();
    const { toasts, off } = collectToasts();
    sendingRef().current = true;

    await expect(send(() => {})).rejects.toThrow("A response is already in progress for this conversation.");
    expect(toasts).toEqual([{ message: "A response is already in progress for this conversation.", tone: "info" }]);
    expect(pipeline).not.toHaveBeenCalled();
    // The pre-existing lock is not stolen by the refused send.
    expect(sendingRef().current).toBe(true);

    off();
    harness.unmount();
  });

  it("refuses a send while an attached run is in flight, without throwing for callers that ignore it", async () => {
    const { harness, sendingRef, send } = setup({ activeRunId: "remote-run" });
    const { toasts, off } = collectToasts();

    await expect(send()).resolves.toBeUndefined();
    expect(toasts).toHaveLength(1);
    expect(pipeline).not.toHaveBeenCalled();
    // The attaching view must not have claimed the send lock.
    expect(sendingRef().current).toBe(false);

    off();
    harness.unmount();
  });

  it("delegates to the pipeline, owns the lock while it runs and releases it after", async () => {
    const release = heldPipeline();
    const { harness, calls, sendingRef, send } = setup();

    const pending = send(() => {});
    expect(sendingRef().current).toBe(true);

    const input = pipeline.mock.calls[0]![0];
    expect(pipeline.mock.calls[0]![1]).toBe(payload);
    expect(input.thread).toBe(thread);
    expect(input.modelId).toBe("model-1");
    expect(input.thinkingLevel).toBe("high");
    expect(input.isCurrentSend()).toBe(true);

    // The pipeline's run-accept writes through while this send is current.
    input.setRecentRun(run);
    expect(calls.recentRuns).toEqual([run]);
    expect(harness.current.localSendRef.current?.runId).toBe("run-1");

    release();
    await pending;

    expect(sendingRef().current).toBe(false);
    expect(harness.current.localSendRef.current).toBeNull();
    // `onAccepted` is the pipeline's business; this hook only forwards it.
    expect(input.onAccepted).toBeDefined();
    harness.unmount();
  });

  it("drops a run-accept that arrives after the send was superseded", async () => {
    const { harness, calls, send } = setup();
    await send();
    const first = pipeline.mock.calls[0]![0];

    // A second send bumps the generation; the first pipeline's late callback
    // must not hijack the view with its own run.
    await send();
    first.setRecentRun({ id: "late-run" } as StoredRun);

    expect(calls.recentRuns).toEqual([]);
    expect(harness.current.localSendRef.current).toBeNull();
    harness.unmount();
  });

  it("surfaces a pipeline failure as an error toast and still releases the lock", async () => {
    pipeline.mockRejectedValue(new Error("attachment too large"));
    const { harness, sendingRef, send } = setup();
    const { toasts, off } = collectToasts();

    await expect(send()).rejects.toThrow("attachment too large");
    expect(toasts).toEqual([{ message: "attachment too large", tone: "error" }]);
    expect(sendingRef().current).toBe(false);

    off();
    harness.unmount();
  });

  it("does not release a lock a newer conversation now owns when an abandoned send settles", async () => {
    // concurrency: unmounting bumps the generation, so the abandoned pipeline's
    // `finally` must leave the new thread's lock alone.
    const release = heldPipeline();
    const { harness, sendingRef, send } = setup();
    const pending = send();
    expect(sendingRef().current).toBe(true);

    act(() => harness.unmount());
    // The next conversation's instance takes the (shared) lock.
    sendingRef().current = true;

    release();
    await pending;
    expect(sendingRef().current).toBe(true);
  });

  it("abandonSend invalidates the in-flight send and frees the lock", async () => {
    const release = heldPipeline();
    const { harness, calls, sendingRef, send } = setup();
    const pending = send();
    const input = pipeline.mock.calls[0]![0];

    act(() => harness.current.abandonSend());
    expect(sendingRef().current).toBe(false);
    expect(harness.current.localSendRef.current).toBeNull();
    expect(input.isCurrentSend()).toBe(false);

    // The late-resolving pipeline can no longer write to the view…
    input.setRecentRun(run);
    expect(calls.recentRuns).toEqual([]);
    // …and its `finally` does not re-take the lock either.
    release();
    await pending;
    expect(sendingRef().current).toBe(false);
    harness.unmount();
  });
});
