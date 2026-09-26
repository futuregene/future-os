// @vitest-environment jsdom
import type { AgentMessage } from "@future-os/thread-projection";
import type { Root } from "react-dom/client";
import type { StoredRun } from "../../integrations/storage/threadStore";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { onFutureEvent } from "../../lib/futureEvents";
import { clearThreadMessageSnapshots } from "./threadMessageCache";
import { useThreadMessages } from "./useThreadMessages";

/**
 * The live-event and loadering-indicator surface of `useThreadMessages`: the
 * `future:agent-event` stream (agent_end / agent_start / compaction_* /
 * user_message), the flash-free loading indicator, and the guards that keep a
 * page read from being applied to a base it no longer matches.
 *
 * The page/cursor behaviour lives in `useThreadMessages.paging.test.tsx`; the
 * warm-snapshot behaviour in `useThreadMessagesHook.test.ts`.
 */
const storage = vi.hoisted(() => ({
  getLatestRun: vi.fn(),
  getRun: vi.fn(),
  getSessionEntriesPage: vi.fn(),
  listRuns: vi.fn(),
}));
const tauri = vi.hoisted(() => ({ invokeCommand: vi.fn() }));

vi.mock("../../integrations/storage/threadStore", () => storage);
vi.mock("../../integrations/tauri/invoke", () => tauri);

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

let current: ReturnType<typeof useThreadMessages>;
let root: Root;
let container: HTMLDivElement;

function Harness({ agentSessionId = "S1", threadId = "T1" }: { agentSessionId?: string | null; threadId?: string | null }) {
  current = useThreadMessages({ threadId, agentSessionId });
  return null;
}

/** A page of user entries, as the agent transcript returns them. */
function history(ids: string[], nextOffset: number, hasMore = true) {
  return {
    entries: ids.map(id => ({
      blocks: [{ kind: "text", text: id }],
      createdAtMs: Date.parse("2026-01-01T00:00:00Z"),
      id,
      kind: "user",
      role: "user",
    })),
    hasMore,
    nextOffset,
  };
}

/**
 * A page whose entries exist but hold no exchange — a session header or a
 * system/meta note. The projection yields no messages from it, which is the
 * shape `loadFromAgent` must report as "loaded, nothing to show" rather than as
 * a failed or empty read.
 */
function nonExchangePage(nextOffset: number, hasMore = true) {
  return {
    entries: [
      { blocks: [], createdAtMs: Date.parse("2026-01-01T00:00:00Z"), id: "sys-1", kind: "system", role: "system" },
      { blocks: [], createdAtMs: Date.parse("2026-01-01T00:00:00Z"), id: "meta-1", kind: "meta" },
    ],
    hasMore,
    nextOffset,
  };
}

const RUN_WINDOW_AT = Date.parse("2026-01-01T00:00:00Z");

/**
 * A page holding a real EXCHANGE: a user entry plus an assistant entry whose
 * reply carries the run id `R-tie`. `history()` deliberately does not do this -
 * it emits user entries only - which is exactly why it cannot observe whether a
 * settled run was stamped onto the reply (see the run-window tests below).
 */
function exchangePage(atMs: number, hasMore = true) {
  return {
    entries: [
      { blocks: [{ kind: "text", text: "u1" }], createdAtMs: atMs, id: "u1", kind: "user", role: "user" },
      { blocks: [{ kind: "text", text: "reply" }], createdAtMs: atMs + 1000, id: "a1", kind: "assistant", role: "assistant", runId: "R-tie" },
    ],
    hasMore,
    nextOffset: 0,
  };
}

/** The settled `runs` row matching `exchangePage`'s reply. */
function failedRun(atMs: number) {
  return {
    createdAt: atMs,
    endedAt: atMs,
    errorMessage: "boom",
    id: "R-tie",
    startedAt: atMs,
    status: "failed",
    threadId: "T1",
    updatedAt: atMs,
  };
}

/** Deliver an agent event to the hook's listener. */
function emit(detail: Record<string, unknown>) {
  act(() => {
    window.dispatchEvent(new CustomEvent("future:agent-event", {
      detail: { sessionId: "S1", threadId: "T1", ...detail },
    }));
  });
}

async function flush() {
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
  });
}

beforeEach(() => {
  clearThreadMessageSnapshots();
  // Explicit, per-test timer reality: the loading-indicator suite switches to
  // fake timers, and a leaked fake clock makes the search's 32ms wait loop and
  // the page promises below never settle (that produced an infinite hang here).
  vi.useRealTimers();
  vi.clearAllMocks();
  storage.getLatestRun.mockResolvedValue(null);
  storage.getRun.mockResolvedValue(null);
  storage.listRuns.mockResolvedValue([]);
  storage.getSessionEntriesPage.mockResolvedValue(history([], 0, false));
  tauri.invokeCommand.mockResolvedValue({ runId: "R1" });
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

async function mount(over: { agentSessionId?: string | null; threadId?: string | null } = {}) {
  await act(async () => root.render(<Harness {...over} />));
  await flush();
}

describe("useThreadMessages agent events", () => {
  it("announces a finished run and re-arms remote detection", async () => {
    const finished: unknown[] = [];
    const off = onFutureEvent("agent_end", () => void finished.push(undefined));
    await mount();

    emit({ eventType: "agent_end", payload: {} });
    expect(finished).toHaveLength(1);

    // After the end, a new remote run may be attached again.
    emit({ eventType: "agent_start", payload: {} });
    await flush();
    expect(tauri.invokeCommand).toHaveBeenCalledWith("attach_remote_stream", { threadId: "T1" });
    off();
  });

  it("attaches a remote run once and reloads the thread it belongs to", async () => {
    await mount();
    storage.getSessionEntriesPage.mockClear();

    emit({ eventType: "agent_start", payload: {} });
    await flush();

    expect(tauri.invokeCommand).toHaveBeenCalledTimes(1);
    // The remote run's first message must appear without a manual reload.
    expect(storage.getSessionEntriesPage).toHaveBeenCalledWith("T1", null);
    expect(storage.getLatestRun).toHaveBeenCalledWith("T1");

    // concurrency: a second start for the same stream is a no-op — the phone
    // driving the session must not cause a second attach.
    emit({ eventType: "agent_start", payload: {} });
    await flush();
    expect(tauri.invokeCommand).toHaveBeenCalledTimes(1);
  });

  it("does not attach a remote run while this view already runs one", async () => {
    // boundary: the local send owns the stream, so a remote-activity start is
    // somebody else's view.
    storage.getLatestRun.mockResolvedValue({ id: "R9", startedAt: 1, status: "running", threadId: "T1" });
    await mount();

    emit({ eventType: "agent_start", payload: {} });
    await flush();
    expect(tauri.invokeCommand).not.toHaveBeenCalled();
  });

  it("re-arms remote detection when the attach call fails", async () => {
    // error-path: the invoke rejects; the next start must be allowed to retry
    // instead of being permanently suppressed.
    tauri.invokeCommand.mockRejectedValue(new Error("no session"));
    await mount();

    emit({ eventType: "agent_start", payload: {} });
    await flush();
    expect(tauri.invokeCommand).toHaveBeenCalledTimes(1);

    emit({ eventType: "agent_start", payload: {} });
    await flush();
    expect(tauri.invokeCommand).toHaveBeenCalledTimes(2);
  });

  it("ignores an attach that resolved without a run", async () => {
    // boundary: the backend attached nothing, so there is no run to reload for.
    tauri.invokeCommand.mockResolvedValue({});
    await mount();
    storage.getSessionEntriesPage.mockClear();

    emit({ eventType: "agent_start", payload: {} });
    await flush();
    expect(tauri.invokeCommand).toHaveBeenCalledTimes(1);
    expect(storage.getSessionEntriesPage).not.toHaveBeenCalled();
  });

  it("ignores events for another thread or another agent session", async () => {
    // boundary: other conversations live on their own keyed instances.
    await mount();
    act(() => {
      window.dispatchEvent(new CustomEvent("future:agent-event", {
        detail: { eventType: "agent_end", payload: {}, sessionId: "OTHER", threadId: "T1" },
      }));
      window.dispatchEvent(new CustomEvent("future:agent-event", {
        detail: { eventType: "agent_end", payload: {}, sessionId: "S1", threadId: "OTHER" },
      }));
      window.dispatchEvent(new CustomEvent("future:agent-event", { detail: undefined }));
    });
    expect(current.messages).toEqual([]);
  });

  it("adds a user bubble from the stream without a reload", async () => {
    await mount();
    emit({
      eventType: "user_message",
      payload: { entry_id: "from-phone", run_id: "R2", text: "typed on the phone" },
    });

    expect(current.messages.map(message => message.content)).toEqual(["typed on the phone"]);
  });

  it("refreshes history for a user event that carries no identity", async () => {
    // boundary: an identity-less event can only invalidate history; its text is
    // not a key, so something else (a rename, a re-route) changed.
    await mount();
    storage.getSessionEntriesPage.mockClear();
    emit({ eventType: "user_message", payload: { text: "no ids here" } });
    await flush();

    expect(storage.getSessionEntriesPage).toHaveBeenCalled();
    expect(current.messages).toEqual([]);
  });

  it("drops a user event that lands while the thread load is still in flight", async () => {
    // concurrency: appending onto a not-yet-committed base would lose it anyway —
    // the authoritative load carries the persisted entry.
    let resolvePage: (value: unknown) => void = () => {};
    storage.getSessionEntriesPage.mockImplementation(() => new Promise((resolve) => {
      resolvePage = resolve;
    }));
    await act(async () => root.render(<Harness />));

    emit({ eventType: "user_message", payload: { entry_id: "e1", text: "during load" } });
    expect(current.messages).toEqual([]);

    await act(async () => {
      resolvePage(history(["persisted"], 0, false));
      await Promise.resolve();
    });
    expect(current.messages.map(message => message.content)).toEqual(["persisted"]);
  });

  it("ignores an unrelated event type", async () => {
    await mount();
    emit({ eventType: "text_chunk", payload: { text: "not ours" } });
    expect(current.messages).toEqual([]);
  });

  it("detaches the event listener on unmount", async () => {
    // concurrency: a late event must not write into a torn-down instance.
    await mount();
    act(() => root.unmount());
    emit({ eventType: "user_message", payload: { entry_id: "late", text: "too late" } });
    expect(current.messages).toEqual([]);
    // Recreate the root so the shared `afterEach` unmount stays valid.
    root = createRoot(container);
  });
});

describe("useThreadMessages standalone compaction", () => {
  function compactionMessages(): AgentMessage[] {
    return current.messages.filter(message => message.segments?.[0]?.kind === "compaction");
  }

  it("projects a started, then committed, session compaction in place", async () => {
    await mount();

    emit({
      eventType: "compaction_started",
      payload: { checkpoint_id: "cp-1", operation_id: "op-1", tokens_before: 1200 },
    });
    expect(compactionMessages()).toHaveLength(1);
    expect(compactionMessages()[0]!.segments![0]).toMatchObject({
      id: "cp-1",
      kind: "compaction",
      status: "running",
      tokensBefore: 1200,
    });

    // The commit updates the SAME bubble rather than adding a second divider.
    emit({
      eventType: "compaction_committed",
      payload: { checkpoint_id: "cp-1", operation_id: "op-1", tokens_after: 300, tokens_before: 1200, trigger: "manual" },
    });
    expect(compactionMessages()).toHaveLength(1);
    expect(compactionMessages()[0]!.segments![0]).toMatchObject({
      tokensAfter: 300,
      tokensBefore: 1200,
      trigger: "manual",
    });
    // The default state is serialized by OMISSION: a committed compaction carries
    // no `status` at all, so a divider persisted by an older build (which never
    // wrote one) reads identically. `running`/`failed` are the explicit ones.
    const segment = compactionMessages()[0]!.segments![0]!;
    expect(segment.kind === "compaction" && "status" in segment).toBe(false);
  });

  it("projects a failed session compaction with its reason", async () => {
    await mount();
    emit({
      eventType: "compaction_failed",
      payload: { error: "context too small", operation_id: "op-1" },
    });

    const segment = compactionMessages()[0]!.segments![0]!;
    expect(segment).toMatchObject({ error: "context too small", status: "failed" });
    // Without a checkpoint id the operation id names the divider.
    expect(segment.kind === "compaction" && segment.id).toBe("op-1");
  });

  it("falls back to a generated id and omits absent counters", async () => {
    // boundary: an event with no optional payload fields must still project a
    // divider rather than throwing or rendering "undefined".
    await mount();
    emit({ eventType: "compaction_started", payload: {} });

    const segment = compactionMessages()[0]!.segments![0]!;
    expect(segment.kind).toBe("compaction");
    if (segment.kind !== "compaction")
      throw new Error("expected a compaction segment");
    expect(segment.id).toMatch(/^session_\d+$/);
    expect(segment.tokensBefore).toBeUndefined();
    expect(segment.tokensAfter).toBeUndefined();
    expect(segment.trigger).toBeUndefined();
    expect(segment.error).toBeUndefined();
    expect(compactionMessages()[0]!.id).toMatch(/^compaction_session_\d+$/);
  });

  it("omits a zero or negative counter instead of rendering it", async () => {
    // boundary: `tokens_after: 0` means "not measured", not "zero tokens".
    await mount();
    emit({
      eventType: "compaction_committed",
      payload: { operation_id: "op-9", tokens_after: 0, tokens_before: -5 },
    });

    const segment = compactionMessages()[0]!.segments![0]!;
    expect(segment.kind === "compaction" && segment.tokensBefore).toBeUndefined();
    expect(segment.kind === "compaction" && segment.tokensAfter).toBeUndefined();
  });

  it("carries an explicit status only for a non-default state", async () => {
    // serialization: `running` and `failed` are stated; `completed` is the
    // omitted default (asserted in the previous test).
    await mount();
    emit({ eventType: "compaction_started", payload: { operation_id: "op-a" } });
    expect(compactionMessages()[0]!.segments![0]).toMatchObject({ status: "running" });

    emit({ eventType: "compaction_failed", payload: { operation_id: "op-b" } });
    expect(compactionMessages()[1]!.segments![0]).toMatchObject({ status: "failed" });
  });

  it("does not project a run-scoped compaction over a live run bubble", async () => {
    // The persisted run event log already carries it; a second divider would
    // duplicate the checkpoint.
    storage.getLatestRun.mockResolvedValue({ id: "R9", startedAt: 1, status: "running", threadId: "T1" });
    await mount();

    emit({ eventType: "compaction_committed", payload: { operation_id: "op-1" } });
    expect(compactionMessages()).toHaveLength(0);
  });

  it("does not project while a remote stream owns the view", async () => {
    await mount();
    emit({ eventType: "agent_start", payload: {} });
    await flush();

    emit({ eventType: "compaction_committed", payload: { operation_id: "op-2" } });
    expect(compactionMessages()).toHaveLength(0);
  });
});

describe("useThreadMessages loading indicator", () => {
  it("holds the indicator off for a load that resolves quickly", async () => {
    vi.useFakeTimers();
    try {
      let resolvePage: (value: unknown) => void = () => {};
      storage.getSessionEntriesPage.mockImplementation(() => new Promise((resolve) => {
        resolvePage = resolve;
      }));
      await act(async () => root.render(<Harness />));
      expect(current.loadingThread).toBe(true);

      // boundary: under the delay the placeholder must never flash.
      await act(async () => {
        await vi.advanceTimersByTimeAsync(199);
      });
      expect(current.loadingIndicator).toBe(false);

      await act(async () => {
        resolvePage(history(["a"], 0, false));
        await Promise.resolve();
      });
      expect(current.loadingThread).toBe(false);
      expect(current.loadingIndicator).toBe(false);
    }
    finally {
      vi.useRealTimers();
    }
  });

  it("shows the indicator after the delay and holds it for its minimum", async () => {
    vi.useFakeTimers();
    try {
      let resolvePage: (value: unknown) => void = () => {};
      storage.getSessionEntriesPage.mockImplementation(() => new Promise((resolve) => {
        resolvePage = resolve;
      }));
      await act(async () => root.render(<Harness />));

      await act(async () => {
        await vi.advanceTimersByTimeAsync(200);
      });
      expect(current.loadingIndicator).toBe(true);

      // The load finishes immediately, but a shown indicator must not flash off.
      await act(async () => {
        resolvePage(history(["a"], 0, false));
        await Promise.resolve();
      });
      expect(current.loadingThread).toBe(false);
      expect(current.loadingIndicator).toBe(true);

      await act(async () => {
        await vi.advanceTimersByTimeAsync(200);
      });
      expect(current.loadingIndicator).toBe(false);
    }
    finally {
      vi.useRealTimers();
    }
  });

  it("never shows the indicator over a warm snapshot", async () => {
    // boundary: a revisiting user sees their messages immediately; a placeholder
    // on top of them would flicker.
    const { setThreadMessageSnapshot } = await import("./threadMessageCache");
    setThreadMessageSnapshot("T1", "S1", [{ content: "cached", id: "c", role: "user" } as AgentMessage]);
    vi.useFakeTimers();
    try {
      let resolvePage: (value: unknown) => void = () => {};
      storage.getSessionEntriesPage.mockImplementation(() => new Promise((resolve) => {
        resolvePage = resolve;
      }));
      await act(async () => root.render(<Harness />));
      expect(current.messages.map(message => message.content)).toEqual(["cached"]);

      await act(async () => {
        await vi.advanceTimersByTimeAsync(500);
      });
      expect(current.loadingIndicator).toBe(false);

      await act(async () => {
        resolvePage(history(["fresh"], 0, false));
        await Promise.resolve();
      });
      expect(current.loadingIndicator).toBe(false);
    }
    finally {
      vi.useRealTimers();
    }
  });

  it("shows nothing for a thread id that is absent", async () => {
    // boundary: no conversation open — nothing to load and nothing to indicate.
    vi.useFakeTimers();
    try {
      await mount({ threadId: null });
      await act(async () => {
        await vi.advanceTimersByTimeAsync(500);
      });
      expect(current.loadingThread).toBe(false);
      expect(current.loadingIndicator).toBe(false);
      expect(current.messages).toEqual([]);
    }
    finally {
      vi.useRealTimers();
    }
  });
});

it("holds a shown indicator for the rest of its minimum, then hides it", async () => {
  // boundary: the load outlasts the hold, so the indicator hides immediately
  // instead of being kept for a duration that has already elapsed.
  vi.useFakeTimers();
  try {
    let resolvePage: (value: unknown) => void = () => {};
    storage.getSessionEntriesPage.mockImplementation(() => new Promise((resolve) => {
      resolvePage = resolve;
    }));
    await act(async () => root.render(<Harness />));

    await act(async () => {
      await vi.advanceTimersByTimeAsync(200);
    });
    expect(current.loadingIndicator).toBe(true);

    // Past the minimum hold, so no remainder is owed.
    await act(async () => {
      await vi.advanceTimersByTimeAsync(300);
    });
    expect(current.loadingIndicator).toBe(true);

    await act(async () => {
      resolvePage(history(["u1"], 0, false));
      await Promise.resolve();
    });
    expect(current.loadingThread).toBe(false);
    expect(current.loadingIndicator).toBe(false);

    // And it stays hidden afterwards, with no stray timer left behind.
    await act(async () => {
      await vi.advanceTimersByTimeAsync(500);
    });
    expect(current.loadingIndicator).toBe(false);
  }
  finally {
    vi.useRealTimers();
  }
});

it("keeps the indicator hidden when the load settles without ever showing it", async () => {
  // boundary: the load resolved inside the delay, so there is nothing to hold
  // and nothing may be left scheduled.
  vi.useFakeTimers();
  try {
    storage.getSessionEntriesPage.mockResolvedValue(history(["u1"], 0, false));
    await act(async () => root.render(<Harness />));
    expect(current.loadingThread).toBe(false);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(1000);
    });
    expect(current.loadingIndicator).toBe(false);
  }
  finally {
    vi.useRealTimers();
  }
});
describe("useThreadMessages page guards", () => {
  it("reports a page with entries but no exchange as loaded-and-empty", async () => {
    // boundary: the agent transcript can hold entries that form no exchange (a
    // session header, a system note). That is a successful read of nothing, not
    // a failure and not a reason to blank the view.
    storage.getSessionEntriesPage.mockResolvedValueOnce(nonExchangePage(7, true));
    await mount();

    expect(current.messages).toEqual([]);
    expect(current.historyError).toBeNull();
    expect(current.hasOlderHistory).toBe(true);
  });

  it("loads the transcript even when the run table cannot be read", async () => {
    // error-path: the run backfill is best-effort; losing it must not lose the
    // transcript (only the Retry/Continue affordances it would have added).
    storage.getSessionEntriesPage.mockResolvedValueOnce(history(["u1", "u2"], 0, false));
    storage.listRuns.mockRejectedValue(new Error("runs table is locked"));
    await mount();

    expect(current.messages.map(message => message.content)).toEqual(["u1", "u2"]);
    expect(current.historyError).toBeNull();
  });

  it("consults the run table when a page carries exchanges", async () => {
    // The filter drops runs outside this page's time window, so only the ones
    // that belong to the page reach the projection.
    storage.getSessionEntriesPage.mockResolvedValueOnce(history(["u1"], 0, false));
    storage.listRuns.mockResolvedValue([
      { createdAt: Date.parse("2026-01-01T00:00:00Z"), id: "in-window", status: "failed", threadId: "T1" },
      { createdAt: Date.parse("2020-01-01T00:00:00Z"), id: "too-old", status: "failed", threadId: "T1" },
    ]);
    await mount();

    expect(storage.listRuns).toHaveBeenCalledWith("T1");
    expect(current.messages.map(message => message.content)).toEqual(["u1"]);
    expect(current.historyError).toBeNull();
  });

  it("stamps a settled run that ends exactly at the page's first message", async () => {
    // boundary: the run-window filter is
    //   (!result.hasMore || (run.endedAt ?? run.updatedAt) >= firstTime) && time <= lastTime
    // A mutation study found the equality case untested: replacing `>=` with `>`
    // left the whole 951-test subtree green, because `history()` (used by every
    // other page fixture here) emits ONLY `role: "user"` entries - with no
    // assistant row there is nothing for `applyRunMetadata` to stamp, so the
    // correct code and the mutant project identically. These tests supply the
    // missing fixture shape (see `exchangePage`/`failedRun` above).
    storage.getSessionEntriesPage.mockResolvedValueOnce(exchangePage(RUN_WINDOW_AT));
    storage.listRuns.mockResolvedValue([failedRun(RUN_WINDOW_AT)]);
    await mount();

    // Included: the reply picks up the run's failure bubble. Under `>` nothing is
    // stamped and only the raw `runId` survives, so this assertion is what kills
    // that mutant.
    const assistant = current.messages.find(message => message.role === "assistant");
    // Pinned to the exact copy rather than `toBeTruthy()`, so a mangled key or a
    // wrong branch cannot pass. Under the `>` mutant these are `undefined`, which is
    // why this line is the mutant's kill site.
    expect(assistant?.terminationNotice).toBe("Please try again later.");
    expect(assistant?.terminationTitle).toBe("Model service error");
  });

  it("drops a run that settled before the page, so the window is not vacuous", async () => {
    // Without this control, a mutant that removed the time filter entirely would
    // still satisfy the test above.
    storage.getSessionEntriesPage.mockResolvedValueOnce(exchangePage(RUN_WINDOW_AT));
    storage.listRuns.mockResolvedValue([failedRun(RUN_WINDOW_AT - 1)]);
    await mount();

    expect(current.messages.find(message => message.role === "assistant")?.terminationNotice).toBeUndefined();
  });

  it("stamps the same tie through the short-circuit when no further pages remain", async () => {
    // The other disjunct: `!result.hasMore` short-circuits the clause, which is
    // also why the tie is unreachable when `hasMore` is false. Guards the
    // direction of the `||` as well as the comparison.
    storage.getSessionEntriesPage.mockResolvedValueOnce(exchangePage(RUN_WINDOW_AT, false));
    storage.listRuns.mockResolvedValue([failedRun(RUN_WINDOW_AT)]);
    await mount();

    expect(current.messages.find(message => message.role === "assistant")?.terminationNotice).toBe("Please try again later.");
  });

  it("folds a still-running run into the page it was reloaded with", async () => {
    // concurrency: the live bubble must land in the SAME commit as history, so
    // opening an active conversation paints both in one frame.
    storage.getLatestRun.mockResolvedValue({ id: "R9", startedAt: 5, status: "running", threadId: "T1" });
    storage.getRun.mockResolvedValue({ id: "R9", startedAt: 5, status: "running", threadId: "T1" });
    storage.getSessionEntriesPage.mockResolvedValue(history(["u1"], 0, false));
    await mount();

    expect(storage.getRun).toHaveBeenCalledWith("R9");
    // Through `recentRun`, so the composer stays locked on the live run.
    expect(current.recentRun?.status).toBe("running");
  });

  it("does not resurrect a bubble for a run that settled during its own read", async () => {
    // concurrency: the run row is the truth, so a settle racing the reload must
    // not paint a streaming bubble for a finished run.
    storage.getLatestRun.mockResolvedValue({ id: "R9", startedAt: 5, status: "running", threadId: "T1" });
    storage.getRun.mockResolvedValue({ id: "R9", status: "completed", threadId: "T1" });
    storage.getSessionEntriesPage.mockResolvedValue(history(["u1"], 0, false));
    await mount();

    expect(storage.getRun).toHaveBeenCalledWith("R9");
    expect(current.messages.map(message => message.content)).toEqual(["u1"]);
  });

  it("still loads history when the live-bubble read fails", async () => {
    // error-path: failing to read the run row must cost only the bubble.
    storage.getLatestRun.mockResolvedValue({ id: "R9", startedAt: 5, status: "running", threadId: "T1" });
    storage.getRun.mockRejectedValue(new Error("run row is unreadable"));
    storage.getSessionEntriesPage.mockResolvedValue(history(["u1"], 0, false));
    await mount();

    expect(current.messages.map(message => message.content)).toEqual(["u1"]);
    expect(current.historyError).toBeNull();
  });

  it("drops a tail read that a newer read superseded while it refreshed", async () => {
    // concurrency: two reloads share one request order, so the older one must not
    // commit its result over the newer one's.
    storage.getSessionEntriesPage.mockResolvedValue(history(["u1"], 0, false));
    await mount();

    let resolveLatest: (value: unknown) => void = () => {};
    storage.getLatestRun.mockImplementation(() => new Promise((resolve) => {
      resolveLatest = resolve;
    }));
    let superseded!: Promise<void>;
    await act(async () => {
      superseded = current.reloadMessagesQuiet("T1", true);
      await Promise.resolve();
    });
    // A second reload takes the tail while the first is still refreshing.
    storage.getLatestRun.mockResolvedValue(null);
    storage.getSessionEntriesPage.mockResolvedValueOnce(history(["newer"], 0, false));
    await act(async () => {
      await current.reloadMessagesQuiet("T1", true);
    });
    storage.getSessionEntriesPage.mockClear();
    await act(async () => {
      resolveLatest(null);
      await superseded;
    });

    // The superseded read never issued its page request.
    expect(storage.getSessionEntriesPage).not.toHaveBeenCalled();
    expect(current.messages.map(message => message.content)).toEqual(["newer"]);
  });

  it("rejects an older page whose cursor did not move", async () => {
    // error-path: a backend that returns the same cursor would loop forever, so
    // the page is refused with a statement of what went wrong.
    storage.getSessionEntriesPage.mockResolvedValueOnce(history(["u3"], 20));
    await mount();
    storage.getSessionEntriesPage.mockResolvedValueOnce(history(["u2"], 20));
    await act(async () => {
      await current.loadOlderHistory();
    });

    expect(current.historyError).toContain("cursor");
    expect(current.messages.map(message => message.content)).toEqual(["u3"]);
  });

  it("adds no rows for an older page that holds no exchange", async () => {
    // boundary: the page at this cursor projects to nothing. It must not invent
    // rows, and it must not surface an error either.
    storage.getSessionEntriesPage.mockResolvedValueOnce(history(["u3"], 20));
    await mount();
    storage.getSessionEntriesPage.mockResolvedValueOnce(nonExchangePage(10, true));

    const beforeCommit = vi.fn();
    await act(async () => {
      await current.loadOlderHistory(beforeCommit);
    });

    // The caller is handed the empty page so it can keep its reading anchor.
    // NOTE: the `status === "empty"` guard that would refuse this page can never
    // fire, because `loadFromAgent` has no producer for that status — finding F5
    // in docs/testing/desktop-agent.md.
    expect(beforeCommit).toHaveBeenCalledWith([]);
    expect(current.historyError).toBeNull();
    expect(current.messages.map(message => message.content)).toEqual(["u3"]);
    expect(current.hasOlderHistory).toBe(true);
  });

  it("hands a loaded older page to the caller before committing it", async () => {
    storage.getSessionEntriesPage.mockResolvedValueOnce(history(["u3", "u4"], 20));
    await mount();
    storage.getSessionEntriesPage.mockResolvedValueOnce(history(["u1", "u2"], 0, false));

    const beforeCommit = vi.fn();
    await act(async () => {
      await current.loadOlderHistory(beforeCommit);
    });

    // The caller sees the page before the commit, so it can move the window
    // anchor in the same paint as the new rows.
    expect(beforeCommit).toHaveBeenCalledWith([
      expect.objectContaining({ content: "u1" }),
      expect.objectContaining({ content: "u2" }),
    ]);
    expect(current.messages.map(message => message.content)).toEqual(["u1", "u2", "u3", "u4"]);
    expect(current.hasOlderHistory).toBe(false);
  });

  it("does not read an older page without a thread, or twice at once", async () => {
    // boundary + concurrency: no thread and a request already in flight are both
    // no-ops, so a scroll gesture cannot queue a dozen page loads.
    await mount({ threadId: null });
    storage.getSessionEntriesPage.mockClear();
    await act(async () => {
      await current.loadOlderHistory();
    });
    expect(storage.getSessionEntriesPage).not.toHaveBeenCalled();

    act(() => root.unmount());
    root = createRoot(container);
    storage.getSessionEntriesPage.mockResolvedValue(history(["u3"], 20));
    await mount();
    storage.getSessionEntriesPage.mockClear();
    let resolvePage: (value: unknown) => void = () => {};
    storage.getSessionEntriesPage.mockImplementation(() => new Promise((resolve) => {
      resolvePage = resolve;
    }));
    await act(async () => {
      const first = current.loadOlderHistory();
      await current.loadOlderHistory();
      expect(storage.getSessionEntriesPage).toHaveBeenCalledTimes(1);
      resolvePage(history(["u2"], 0, false));
      await first;
    });
  });

  it("reports a failed page read on the thread without losing what is shown", async () => {
    storage.getSessionEntriesPage.mockResolvedValueOnce(history(["u3"], 20));
    await mount();
    storage.getSessionEntriesPage.mockRejectedValueOnce(new Error("read failed"));

    await act(async () => {
      await current.loadOlderHistory();
    });
    expect(current.historyError).toBe("read failed");
    expect(current.messages.map(message => message.content)).toEqual(["u3"]);
    expect(current.hasOlderHistory).toBe(true);
  });

  it("retries the history read after a failure", async () => {
    storage.getSessionEntriesPage.mockRejectedValueOnce(new Error("transient"));
    await mount();
    expect(current.historyError).toBe("transient");

    storage.getSessionEntriesPage.mockResolvedValueOnce(history(["recovered"], 0, false));
    await act(async () => {
      await current.reloadMessagesQuiet("T1", true);
    });
    expect(current.historyError).toBeNull();
    expect(current.messages.map(message => message.content)).toEqual(["recovered"]);
  });

  it("ignores a stale run-status refresh from a replaced conversation", async () => {
    // concurrency: a session replacement invalidates the outgoing owner's
    // callbacks, so its run read must not write into the new conversation.
    await mount({ agentSessionId: "S1" });
    const staleRefresh = current.refreshRecentRun;

    await mount({ agentSessionId: "S2" });
    storage.getLatestRun.mockClear();
    await act(async () => {
      await staleRefresh("T1");
    });

    expect(storage.getLatestRun).not.toHaveBeenCalled();
  });
});

it("ignores a tail read started by a conversation that was replaced", async () => {
  // concurrency: the replacement bumps the source version, so a read captured
  // from the outgoing conversation must not touch the new one.
  await mount({ agentSessionId: "S1" });
  const staleReload = current.reloadMessagesQuiet;

  await mount({ agentSessionId: "S2" });
  storage.getSessionEntriesPage.mockClear();
  await act(async () => {
    await staleReload("T1", true);
  });

  expect(storage.getSessionEntriesPage).not.toHaveBeenCalled();
});
it("does not report a failed page read that a replaced conversation superseded", async () => {
  // concurrency: a page that fails after a session replacement must not surface
  // its error on the thread that replaced it.
  //
  // Measured note, because my first comment here claimed the wrong mechanism: the
  // swallow does NOT come from the epoch re-check inside the catch (that arm is
  // unreachable - see §5). `loadFromAgent` never throws, it catches everything and
  // returns `{ status: "failed" }`, so the rejection resurfaces as a *result* and
  // the post-await check `if (epoch !== historyEpochRef.current) return;` returns
  // before the status is even inspected. This test therefore pins the behaviour
  // through that earlier guard; the catch's own check is dead code, and it is
  // recorded as such rather than credited to this test.
  storage.getSessionEntriesPage.mockResolvedValueOnce(history(["u3"], 20));
  await mount({ agentSessionId: "S1" });

  let failPage!: (error: unknown) => void;
  storage.getSessionEntriesPage.mockImplementationOnce(
    () => new Promise((_resolve, reject) => {
      failPage = reject;
    }),
  );
  const pending = current.loadOlderHistory();

  // Replace the conversation while the page is still in flight.
  storage.getSessionEntriesPage.mockResolvedValueOnce(history(["u9"], 0, false));
  await mount({ agentSessionId: "S2" });
  expect(current.messages.map(message => message.content)).toContain("u9");

  failPage(new Error("stale read failed"));
  await act(async () => {
    await pending;
  });

  // The read did reject and its failure was swallowed - so this test cannot pass
  // vacuously - yet nothing was reported on the replacing conversation.
  expect(current.historyError).toBeNull();
  expect(current.messages.map(message => message.content)).toContain("u9");
});
it("ignores a run cached by a conversation that was replaced", async () => {
  // concurrency: `setRecentRun` is exposed to the parent, so a parent holding the
  // outgoing conversation's callback can call it after a session replacement. The
  // guard must drop it rather than let one conversation's run row become the
  // other's "recent run". `setRecentRun`'s closure captures its own `source`,
  // which is what the guard compares against `sourceRef.current` - the sibling
  // test above covers the same shape for `refreshRecentRun`'s async read, this one
  // for the synchronous cache write.
  await mount({ agentSessionId: "S1" });
  const staleSetRecentRun = current.setRecentRun;
  // `failedRun` is a partial fixture (it omits the model/trigger columns, which
  // `recentRun` carries through unchanged), so it needs the same kind of cast the
  // neighbouring attachment-fixture test uses to stand in for a stored row.
  const run = failedRun(RUN_WINDOW_AT) as unknown as StoredRun;

  await mount({ agentSessionId: "S2" });
  expect(current.recentRun).toBeNull();

  // The replacing conversation's own setter works - so the assertion below is
  // about staleness, not about writes being ignored wholesale.
  act(() => current.setRecentRun(run));
  expect(current.recentRun).toEqual(run);

  // The outgoing conversation's captured setter is refused.
  act(() => staleSetRecentRun(failedRun(RUN_WINDOW_AT + 60_000) as unknown as StoredRun));
  expect(current.recentRun).toEqual(run);
});

describe("useThreadMessages full-history search", () => {
  it("refuses to search when no page has ever loaded", async () => {
    // error-path: searching an uncached thread would silently return nothing, so
    // the caller is told history is unavailable instead.
    storage.getSessionEntriesPage.mockRejectedValueOnce(new Error("offline"));
    await mount();

    await act(async () => {
      await expect(current.loadAllHistoryForSearch(new AbortController().signal))
        .rejects
        .toThrow("Thread history is unavailable for search.");
    });
  });

  it("waits for an in-flight thread load before searching", async () => {
    // concurrency: searching mid-load would walk a base that is about to be
    // replaced, so the search waits for the load to commit first.
    //
    // The search must NOT be awaited inside the same `act` that has to flush the
    // load's commit: React defers the state flush until the act callback
    // resolves, while the search is waiting for that very flush — awaiting both
    // together deadlocks. Start it, let act flush, then await it.
    const pending: ((value: unknown) => void)[] = [];
    storage.getSessionEntriesPage.mockImplementation(() => new Promise((resolve) => {
      pending.push(resolve);
    }));
    await act(async () => root.render(<Harness />));
    expect(current.loadingThread).toBe(true);
    expect(pending.length).toBeGreaterThan(0);

    let search!: Promise<AgentMessage[]>;
    await act(async () => {
      search = current.loadAllHistoryForSearch(new AbortController().signal);
      const page = history(["u1"], 0, false);
      storage.getSessionEntriesPage.mockResolvedValue(page);
      pending.forEach(resolve => resolve(page));
      await Promise.resolve();
    });

    const messages = await search;
    expect(messages.map(message => message.content)).toEqual(["u1"]);
    expect(current.loadingThread).toBe(false);
    expect(current.historyError).toBeNull();
  });

  it("waits for an older page already in flight, then finishes the search", async () => {
    // concurrency: two readers of the same cursor would double-page, so the
    // search yields to the one already running and then continues from it.
    storage.getSessionEntriesPage.mockResolvedValueOnce(history(["u3"], 20));
    await mount();

    let resolveOlder: (value: unknown) => void = () => {};
    storage.getSessionEntriesPage.mockImplementationOnce(() => new Promise((resolve) => {
      resolveOlder = resolve;
    }));

    let older!: Promise<void>;
    let search!: Promise<AgentMessage[]>;
    await act(async () => {
      older = current.loadOlderHistory();
      await Promise.resolve();
      search = current.loadAllHistoryForSearch(new AbortController().signal);
      // Let the search reach its wait, then hand the page to the in-flight read.
      setTimeout(() => resolveOlder(history(["u1", "u2"], 0, false)), 20);
      await older;
    });

    const messages = await search;
    expect(messages.map(message => message.content)).toEqual(["u1", "u2", "u3"]);
    expect(current.hasOlderHistory).toBe(false);
  });

  it("stops a full-history search when the pages cannot advance", async () => {
    // error-path: a search that cannot make progress must surface the failure
    // rather than return a partial result set.
    storage.getSessionEntriesPage.mockResolvedValueOnce(history(["u3"], 20));
    await mount();
    storage.getSessionEntriesPage.mockRejectedValue(new Error("offline"));

    await act(async () => {
      await expect(current.loadAllHistoryForSearch(new AbortController().signal)).rejects.toThrow("cursor");
    });
    expect(current.hasOlderHistory).toBe(true);
  });

  it("re-checks liveness while waiting for the thread load", async () => {
    // concurrency: the wait between polls re-checks the signal, so a search
    // cancelled during the wait stops instead of walking a torn-down thread.
    const pending: ((value: unknown) => void)[] = [];
    storage.getSessionEntriesPage.mockImplementation(() => new Promise((resolve) => {
      pending.push(resolve);
    }));
    await act(async () => root.render(<Harness />));

    const controller = new AbortController();
    let search!: Promise<AgentMessage[]>;
    await act(async () => {
      search = current.loadAllHistoryForSearch(controller.signal);
      controller.abort();
      await Promise.resolve();
    });

    await expect(search).rejects.toMatchObject({ name: "AbortError" });
  });
});

it("restarts the walk when another read replaces the base mid-search", async () => {
  // concurrency: a tail read that commits while the search is paging replaces
  // the array the search is extending, so the search re-reads from the start
  // (up to three times) instead of appending onto a base it no longer owns.
  storage.getSessionEntriesPage.mockResolvedValueOnce(history(["u3"], 20));
  await mount();

  let resolveOlder: (value: unknown) => void = () => {};
  storage.getSessionEntriesPage.mockImplementationOnce(() => new Promise((resolve) => {
    resolveOlder = resolve;
  }));

  let search!: Promise<AgentMessage[]>;
  await act(async () => {
    search = current.loadAllHistoryForSearch(new AbortController().signal);
    // Let the search issue its older-page read…
    await Promise.resolve();
    // …then replace the base under it and hand it the now-stale page.
    storage.getSessionEntriesPage.mockResolvedValueOnce(history(["u3"], 20));
    await current.reloadMessagesQuiet("T1", true);
    // The second pass of the walk gets a page that does advance the cursor.
    storage.getSessionEntriesPage.mockResolvedValueOnce(history(["u1", "u2"], 0, false));
    resolveOlder(history(["u1"], 0, false));
    await Promise.resolve();
  });

  const messages = await search;
  // The restart is observable: the search returned the rows from its second
  // pass, not the discarded page's.
  expect(messages.map(message => message.content)).toContain("u2");
});
describe("useThreadMessages history cache hand-off", () => {
  it("serves the next mount of the same session from the warm cache", async () => {
    // serialization: the cache is keyed by (thread, agent session), so a revisit
    // paints the known rows immediately while the authoritative read runs.
    storage.getSessionEntriesPage.mockResolvedValueOnce(history(["u1", "u2"], 0, false));
    await mount();
    expect(current.messages.map(message => message.content)).toEqual(["u1", "u2"]);
    act(() => root.unmount());

    let resolvePage: (value: unknown) => void = () => {};
    storage.getSessionEntriesPage.mockImplementation(() => new Promise((resolve) => {
      resolvePage = resolve;
    }));
    root = createRoot(container);
    await act(async () => root.render(<Harness />));
    expect(current.messages.map(message => message.content)).toEqual(["u1", "u2"]);

    await act(async () => {
      resolvePage(history(["u1", "u2", "u3"], 0, false));
      await Promise.resolve();
    });
    expect(current.messages.map(message => message.content)).toEqual(["u1", "u2", "u3"]);
  });

  it("keeps the previous conversation's rows out of a newly opened thread", async () => {
    // boundary: the cache key includes the agent session, so binding a different
    // one must not show the other conversation's messages.
    storage.getSessionEntriesPage.mockResolvedValueOnce(history(["old-thread"], 0, false));
    await mount();

    storage.getSessionEntriesPage.mockResolvedValueOnce(history(["new-thread"], 0, false));
    await mount({ agentSessionId: "S2" });
    expect(current.messages.map(message => message.content)).toEqual(["new-thread"]);
  });
});
