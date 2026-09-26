// @vitest-environment jsdom
import type { MutableRefObject } from "react";
import { act, useRef } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { renderHook } from "../../test/renderHook";
import { resetRunProjection, upsertStreamingPreview } from "./threadRunProjection";
import { useRunReattach } from "./useRunReattach";

vi.mock("./threadRunProjection", async (importOriginal) => {
  const actual = await importOriginal<typeof import("./threadRunProjection")>();
  return {
    ...actual,
    resetRunProjection: vi.fn(),
    upsertStreamingPreview: vi.fn(async () => {}),
  };
});

const listeners = vi.hoisted(() => new Map<string, Set<(event: { payload: unknown }) => void>>());
vi.mock("@tauri-apps/api/event", () => ({
  listen: async (name: string, handler: (event: { payload: unknown }) => void) => {
    const handlers = listeners.get(name) ?? new Set();
    handlers.add(handler);
    listeners.set(name, handlers);
    return () => handlers.delete(handler);
  },
}));

const project = vi.mocked(upsertStreamingPreview);
const resetProjection = vi.mocked(resetRunProjection);

function setup(overrides: Partial<Parameters<typeof useRunReattach>[0]> = {}) {
  const refreshRecentRun = vi.fn(async () => {});
  const reloadMessagesQuiet = vi.fn(async () => {});
  const setMessages = vi.fn();
  let sending!: MutableRefObject<boolean>;
  const harness = renderHook(() => {
    const sendingRef = useRef(false);
    sending = sendingRef;
    return useRunReattach({
      threadId: "T1",
      workspaceId: "W1",
      activeRunId: "R1",
      activeRunStartedAt: 1_000,
      loadingThread: false,
      sendingRef,
      setMessages,
      refreshRecentRun,
      reloadMessagesQuiet,
      ...overrides,
    });
  });
  return { harness, refreshRecentRun, reloadMessagesQuiet, setMessages, sending: () => sending };
}

/** Deliver a Tauri event to every subscriber of `name`. */
function emit(name: string, payload: unknown) {
  act(() => {
    for (const handler of listeners.get(name) ?? [])
      handler({ payload });
  });
}

function running(over: Record<string, unknown> = {}) {
  return {
    updates: [{ threadId: "T1", runId: "R1", revision: 1, status: "running", resetProjection: false, ...over }],
  };
}

/** Let the mount-time tick and its registration-race re-arm both land. */
async function settleTicks() {
  await act(async () => {
    await vi.advanceTimersByTimeAsync(200);
  });
  return vi.mocked(upsertStreamingPreview).mock.calls.length;
}

beforeEach(() => {
  vi.useFakeTimers();
  vi.clearAllMocks();
  listeners.clear();
});

describe("useRunReattach runtime updates", () => {
  it("drops a reset flag for a live push and keeps streaming", async () => {
    const { harness } = setup();
    const baseline = await settleTicks();

    emit("thread-runtime-updated", running({ resetProjection: true }));
    await settleTicks();

    expect(resetProjection).toHaveBeenCalledWith("R1");
    // The push re-arms the live preview rather than tearing it down.
    expect(project.mock.calls.length).toBe(baseline + 1);
    harness.unmount();
  });

  it.each(["completed", "failed", "cancelled"] as const)(
    "stops previewing and force-reloads when the run reports %s",
    async (status) => {
      const { harness, refreshRecentRun, reloadMessagesQuiet } = setup();
      const baseline = await settleTicks();

      emit("thread-runtime-updated", running({ status }));
      await settleTicks();

      expect(refreshRecentRun).toHaveBeenCalledWith("T1", "W1");
      // `true` = force: the persisted message must replace the synthetic bubble.
      expect(reloadMessagesQuiet).toHaveBeenCalledWith("T1", true);
      // A settled run never projects another preview.
      expect(project.mock.calls.length).toBe(baseline);
      harness.unmount();
    },
  );

  it("ignores pushes for another thread or another run", async () => {
    const { harness, refreshRecentRun, reloadMessagesQuiet } = setup();
    const baseline = await settleTicks();

    emit("thread-runtime-updated", { updates: [{ threadId: "T2", runId: "R1", status: "completed" }] });
    emit("thread-runtime-updated", { updates: [{ threadId: "T1", runId: "R9", status: "completed" }] });
    emit("thread-runtime-updated", { updates: [] });
    await settleTicks();

    expect(refreshRecentRun).not.toHaveBeenCalled();
    expect(reloadMessagesQuiet).not.toHaveBeenCalled();
    expect(project.mock.calls.length).toBe(baseline);
    harness.unmount();
  });

  it("self-heals a lost terminal push on the 30s pass, but never during a local send", async () => {
    const { harness, refreshRecentRun, sending } = setup();
    await act(async () => {});

    await act(async () => {
      await vi.advanceTimersByTimeAsync(30_000);
    });
    expect(refreshRecentRun).toHaveBeenCalledWith("T1", "W1");

    // While this view's own send owns the stream the pass is redundant.
    refreshRecentRun.mockClear();
    sending().current = true;
    await act(async () => {
      await vi.advanceTimersByTimeAsync(30_000);
    });
    expect(refreshRecentRun).not.toHaveBeenCalled();
    harness.unmount();
  });

  it("picks up a remote-driven run from the remote-activity signal", async () => {
    const { harness, refreshRecentRun, reloadMessagesQuiet } = setup();
    await settleTicks();

    // Another conversation's activity is none of this view's business.
    emit("remote-activity", "T2");
    expect(refreshRecentRun).not.toHaveBeenCalled();

    emit("remote-activity", "T1");
    expect(refreshRecentRun).toHaveBeenCalledWith("T1", "W1");
    // No force: the phone's user bubble must show without discarding the
    // streaming baseline this view may already hold.
    expect(reloadMessagesQuiet).toHaveBeenCalledWith("T1");
    harness.unmount();
  });

  it("ignores remote activity while a local send owns the view", async () => {
    const { harness, refreshRecentRun, reloadMessagesQuiet, sending } = setup();
    await act(async () => {});
    sending().current = true;

    emit("remote-activity", "T1");
    expect(refreshRecentRun).not.toHaveBeenCalled();
    expect(reloadMessagesQuiet).not.toHaveBeenCalled();
    harness.unmount();
  });

  it("force-reloads once a previewed run settles, unless a local send owns the view", async () => {
    // The synthetic streaming bubble must be replaced by the persisted message
    // the moment the run this view was previewing reports it has settled.
    const attached = { activeRunId: "R1" as string | null };
    const previewed = setup(attached);
    await settleTicks();
    expect(previewed.reloadMessagesQuiet).not.toHaveBeenCalled();

    attached.activeRunId = null;
    previewed.harness.rerender();
    expect(previewed.reloadMessagesQuiet).toHaveBeenCalledWith("T1", true);
    previewed.harness.unmount();

    // While this view's own send owns the stream, the send path renders the
    // final state itself — a second reload would race it.
    const owned = { activeRunId: "R1" as string | null };
    const sending = setup(owned);
    await settleTicks();
    owned.activeRunId = null;
    sending.sending().current = true;
    sending.harness.rerender();
    expect(sending.reloadMessagesQuiet).not.toHaveBeenCalled();
    sending.harness.unmount();
  });

  it("registers no listeners at all without a thread", async () => {
    // boundary: no conversation is open yet, so every effect must bail out.
    const { harness, refreshRecentRun, reloadMessagesQuiet } = setup({ threadId: null, activeRunId: null });
    await act(async () => {});

    expect(listeners.get("remote-activity")).toBeUndefined();
    expect(listeners.get("thread-runtime-updated")).toBeUndefined();
    expect(refreshRecentRun).not.toHaveBeenCalled();
    expect(reloadMessagesQuiet).not.toHaveBeenCalled();
    harness.unmount();
  });

  it("does not attach at all while the view is detached or its history is loading", async () => {
    const detached = setup({ activeRunId: null });
    await act(async () => {});
    expect(project).not.toHaveBeenCalled();
    expect(listeners.get("thread-runtime-updated")).toBeUndefined();
    detached.harness.unmount();

    const loading = setup({ loadingThread: true });
    await act(async () => {});
    expect(project).not.toHaveBeenCalled();
    loading.harness.unmount();
  });

  it("sees a push that lands before the listener registration resolves", async () => {
    // concurrency: the registration race — the tick re-arms once `listen`
    // resolves, so no push between the first read and the subscription is lost.
    const { harness } = setup();
    const baseline = await settleTicks();
    expect(baseline).toBe(2);
    harness.unmount();
  });

  it("does not re-arm the tick when the registration resolves after unmount", async () => {
    // concurrency: `listen` is an async IPC call, so the view can already be gone
    // when it resolves (a fast thread switch). The registration echo must not touch
    // a stopped tick whose view unmounted. The sibling test above covers the echo
    // landing while mounted; this covers the same `then` with `cancelled` true, which
    // no test reached because every one of them awaited the registration first.
    const { harness } = setup();
    harness.unmount();
    await act(async () => {
      await vi.advanceTimersByTimeAsync(200);
    });

    // Unmounting stopped the tick, so the registration echo must not add a second
    // projection: the only call is the attach-time `liveTick.request()` the effect
    // makes unconditionally.
    //
    // Measured note: this arm is now covered but does NOT discriminate the guard.
    // Replacing `!cancelled` with `true` leaves this test passing, because the tick
    // refuses the request anyway — `liveStreamTick`'s `request()` returns early once
    // `stop()` has run, and the cleanup that sets `cancelled` also calls
    // `liveTick.stop()` (and the drain re-checks the same flag via `isActive`). The
    // guard is therefore triply redundant; recorded as finding F12, and the test is
    // kept as the only driver of the "unmounted during registration" state.
    expect(project).toHaveBeenCalledTimes(1);
  });

  it("detaches the listener, the tick and the self-heal pass on unmount", async () => {
    const { harness, refreshRecentRun } = setup();
    const baseline = await settleTicks();
    harness.unmount();
    await act(async () => {});
    expect(listeners.get("thread-runtime-updated")?.size ?? 0).toBe(0);
    expect(listeners.get("remote-activity")?.size ?? 0).toBe(0);

    emit("thread-runtime-updated", running({ status: "completed" }));
    await act(async () => {
      await vi.advanceTimersByTimeAsync(60_000);
    });
    expect(refreshRecentRun).not.toHaveBeenCalled();
    expect(project.mock.calls.length).toBe(baseline);
  });
});
