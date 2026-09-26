import { createElement } from "react";
import { AppState } from "react-native";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import type { RemoteClient } from "../client";
import { applyStreamEvent, applyStreamEvents, commitAcknowledgedUserMessage, emptyTimeline } from "../timeline";
import type { HistoryEntry, StreamEvent } from "../types";
import { useTimelineController } from "../useTimelineController";

type Options = Parameters<typeof useTimelineController>[0];
type Result = ReturnType<typeof useTimelineController>;

function userEntry(id: string, text: string): HistoryEntry {
  return {
    id,
    kind: "user",
    role: "user",
    createdAtMs: 0,
    blocks: [{ kind: "text", text }],
  };
}
function assistantEntry(
  id: string,
  text: string,
  runId?: string,
): HistoryEntry {
  return {
    id,
    role: "assistant",
    kind: "assistant",
    createdAtMs: 0,
    blocks: [{ kind: "text", text }],
    runId,
  };
}
function evt(
  type: string,
  data: string,
  runId?: string,
  idx?: number,
): StreamEvent {
  return {
    type,
    data,
    ...(runId ? { runId } : {}),
    ...(idx != null ? { idx } : {}),
  };
}

describe("useTimelineController", () => {
  let options: Options;
  let result: { current: Result };
  let renderer: ReactTestRenderer | null;
  let request: jest.Mock;

  function Harness(): null {
    result.current = useTimelineController(options);
    return null;
  }

  function render(): void {
    if (options.selectedSessionId)
      options.selectedRef.current = options.selectedSessionId;
    act(() => {
      renderer = create(createElement(Harness));
    });
  }

  async function flush(times = 40): Promise<void> {
    await act(async () => {
      for (let i = 0; i < times; i += 1) {
        await Promise.resolve();
      }
    });
  }

  function client(): { current: RemoteClient | null } {
    return options.clientRef;
  }

  function makeClient(): RemoteClient {
    return {
      requestRetry: request,
      recoverNow: jest.fn(async () => {}),
    } as unknown as RemoteClient;
  }

  beforeEach(() => {
    request = jest.fn();
    options = {
      clientRef: { current: makeClient() },
      selectedRef: { current: "" },
      selectedSessionId: "",
      draft: false,
      refreshModels: jest.fn(async () => {}),
      refreshSessions: jest.fn(async () => {}),
      setTitleOverrides: jest.fn(),
    };
    result = { current: undefined as unknown as Result };
    renderer = null;
  });

  afterEach(() => {
    if (renderer) {
      act(() => renderer!.unmount());
      renderer = null;
    }
  });

  /** Establish a session timeline by driving an "open" reconcile through the engine. */
  async function establish(sessionId = "s1"): Promise<void> {
    options.selectedRef.current = sessionId;
    act(() => {
      result.current.reconcileSession(sessionId, "open");
    });
    await flush();
  }

  test("state reads refresh the selected model after reconnect without replaying settings as chat", async () => {
    options.onSessionState = jest.fn();
    let model = "p/old";
    request.mockImplementation(async (command: { type: string }) => ({ data:
      command.type === "get_state" ? { model, thinkingLevel: "high" } : { entries: [] },
    }));
    render();
    await establish();
    expect(options.onSessionState).toHaveBeenLastCalledWith("s1", { model: "p/old", thinkingLevel: "high" });
    model = "p/new";
    act(() => result.current.reconcileSession("s1", "reconnect"));
    await flush();
    expect(options.onSessionState).toHaveBeenLastCalledWith("s1", { model: "p/new", thinkingLevel: "high" });
  });

  test("a state request started before a live settings event cannot overwrite it", async () => {
    options.onSessionState = jest.fn();
    let resolve!: (value: unknown) => void;
    request.mockImplementation((command: { type: string }) => command.type === "get_state"
      ? new Promise(yes => { resolve = yes; }) : Promise.resolve({ data: { entries: [] } }));
    render();
    await establish();
    act(() => result.current.handleEvent(evt("model_changed", '{"model":"p/new"}'), "s1"));
    await act(async () => { resolve({ data: { model: "p/old" } }); });
    await flush();
    expect(options.onSessionState).not.toHaveBeenCalled();
  });

  test("background grace stops projection and foreground recovery restores missed text", async () => {
    jest.useFakeTimers();
    const originalActivity = Object.getOwnPropertyDescriptor(AppState, "currentState")!;
    const setActivity = (value: string) => Object.defineProperty(AppState, "currentState", { configurable: true, value });
    setActivity("active");
    const events = [evt("agent_start", "{}", "r", 0), evt("text_chunk", '{"text":"prefix"}', "r", 1)];
    request.mockImplementation(async (command: { type: string; sinceIdx?: number }) => ({ data:
      command.type === "get_state" ? { activeRun: { runId: "r" } }
        : command.type === "get_session_entries" ? { entries: [] }
          : { events: events.filter(event => event.idx! > (command.sinceIdx ?? -1)) },
    }));
    options.selectedSessionId = "s1";
    try {
      render();
      await establish();
      const before = result.current.timeline;
      setActivity("background");
      const tail = evt("text_chunk", '{"text":" while hidden"}', "r", 2);
      events.push(tail);
      act(() => result.current.handleEvent(tail, "s1"));
      await act(async () => { await jest.advanceTimersByTimeAsync(1000); });
      expect(result.current.timeline).toBe(before);
      const reads = request.mock.calls.length;
      act(() => result.current.reconcileSession("s1", "reconnect"));
      await flush();
      expect(request).toHaveBeenCalledTimes(reads);
      setActivity("active");
      act(() => result.current.reconcileSession("s1", "reconnect"));
      await flush();
      expect(result.current.timeline.items.at(-1)).toMatchObject({ text: "prefix while hidden" });
    } finally { Object.defineProperty(AppState, "currentState", originalActivity); jest.useRealTimers(); }
  });

  test("cache eviction preserves a queued timeline commit before ref effects run", async () => {
    options.selectedSessionId = "s1";
    render();
    const engine = result.current.syncEngineRef.current!;
    const prune = jest.spyOn(engine, "pruneCache").mockReturnValue(["old"]);
    // The hook's subscriber queues setTimelines first. Trigger cleanup in the
    // same engine commit, before React has mirrored that new state to its ref.
    const unsubscribe = engine.subscribe(() => result.current.prepareTimelineOpen("s1"));
    try {
      act(() => engine.mutate("s1", () => ({ ...emptyTimeline(), items: [
        { kind: "message", id: "fresh", role: "user", text: "newly committed" },
      ] })));
      await flush();
      expect(result.current.timelinePending).toBe(false);
      expect(result.current.timeline.items[0]).toMatchObject({ id: "fresh", text: "newly committed" });
    } finally { unsubscribe(); prune.mockRestore(); }
  });

  test("navigation evicts inactive UI history and paging state, then reloads it on demand", async () => {
    request.mockImplementation(
      async (command: { type: string; sessionId: string }) => ({
        data:
          command.type === "get_state"
            ? {}
            : {
                entries: [userEntry(command.sessionId, "history")],
                hasMore: true,
                nextOffset: 10,
              },
      }),
    );
    options.selectedSessionId = "s0";
    render();
    for (let i = 0; i < 12; i++) {
      const id = `s${i}`;
      options.selectedSessionId = id;
      options.selectedRef.current = id;
      await act(async () => {
        result.current.prepareTimelineOpen(id);
        renderer!.update(createElement(Harness));
        await result.current.syncEngineRef.current!.open(id);
      });
      await flush();
    }
    expect(result.current.syncEngineRef.current!.timelineFor("s0")).toBeNull();
    options.selectedSessionId = "s0";
    options.selectedRef.current = "s0";
    act(() => {
      result.current.prepareTimelineOpen("s0");
      renderer!.update(createElement(Harness));
    });
    expect(result.current.timelinePending).toBe(true);
    expect(result.current.canLoadOlderTimeline).toBe(false);
    await act(async () => {
      await result.current.syncEngineRef.current!.open("s0");
    });
    await flush();
    expect(result.current.timelinePending).toBe(false);
    expect(result.current.canLoadOlderTimeline).toBe(true);
  });

  test("back navigation defers cache sizing and cancels stale cleanup on reopen", async () => {
    jest.useFakeTimers();
    request.mockImplementation(async (command: { type: string }) => ({
      data: command.type === "get_state" ? {} : { entries: [userEntry("u", "history")] },
    }));
    options.selectedSessionId = "s1";
    try {
      render();
      await establish();
      await act(async () => { await jest.advanceTimersByTimeAsync(0); });
      const prune = jest.spyOn(result.current.syncEngineRef.current!, "pruneCache");
      const navigate = (id: string) => {
        options.selectedSessionId = id;
        options.selectedRef.current = id;
        act(() => renderer!.update(createElement(Harness)));
      };
      navigate("");
      expect(result.current.timeline.items).toHaveLength(0);
      expect(prune).not.toHaveBeenCalled();
      navigate("s1");
      expect(prune).not.toHaveBeenCalled();
      await act(async () => { await jest.advanceTimersByTimeAsync(0); });
      expect(prune.mock.calls).toEqual([["s1"]]);
      navigate("");
      expect(prune).toHaveBeenCalledTimes(1);
      await act(async () => { await jest.advanceTimersByTimeAsync(0); });
      expect(prune.mock.calls).toEqual([["s1"], [""]]);
      navigate("s1");
      act(() => renderer!.unmount());
      renderer = null;
      await jest.advanceTimersByTimeAsync(0);
      expect(prune).toHaveBeenCalledTimes(2);
    } finally { jest.useRealTimers(); }
  });

  test("slow active-run replay does not trigger the 15-second history timeout", async () => {
    jest.useFakeTimers();
    options.selectedSessionId = "s1";
    options.selectedRef.current = "s1";
    request.mockImplementation(async (command: { type: string }) => {
      if (command.type === "get_state")
        return { data: { activeRun: { runId: "r" } } };
      if (command.type === "get_session_entries")
        return { data: { entries: [userEntry("u", "readable history")] } };
      await new Promise((resolve) => setTimeout(resolve, 20_000));
      return { data: { events: [] } };
    });
    try {
      render();
      await establish();
      expect(result.current.timelinePending).toBe(false);
      expect(result.current.timeline.items).toHaveLength(1);
      expect(result.current.timelineSyncStatus).toBe("syncing");
      await act(async () => {
        await jest.advanceTimersByTimeAsync(15_001);
      });
      expect(result.current.timelinePending).toBe(false);
      expect(result.current.timelineError).toBeNull();
      expect(result.current.timeline.items[0]).toMatchObject({
        id: "m_u",
        text: "readable history",
      });
      expect(result.current.timelineSyncStatus).toBe("syncing");
      await act(async () => {
        await jest.advanceTimersByTimeAsync(5_000);
      });
      expect(result.current.timelineError).toBeNull();
      expect(result.current.timelineSyncStatus).toBe("idle");
    } finally {
      act(() => renderer!.unmount());
      renderer = null;
      jest.useRealTimers();
    }
  });

  test.each(["restart", "resend"])(
    "%s bridges a disjoint tail without evicting already displayed history",
    async (mode) => {
      options.selectedSessionId = "s1";
      options.selectedRef.current = "s1";
      const exchanges = (start: number, end: number) =>
        Array.from({ length: end - start + 1 }, (_, i) => start + i).flatMap(
          (n) => [
            userEntry(`u${n}`, `user ${n}`),
            assistantEntry(`a${n}`, `answer ${n}`),
          ],
        );
      request
        .mockResolvedValueOnce({ success: true, data: {} })
        .mockResolvedValueOnce({
          success: true,
          data: { entries: exchanges(11, 20), hasMore: true, nextOffset: 20 },
        })
        .mockResolvedValueOnce({
          success: true,
          data: { entries: exchanges(1, 10), hasMore: false, nextOffset: 0 },
        })
        .mockResolvedValueOnce({ success: true, data: {} })
        .mockResolvedValueOnce({
          success: true,
          data: { entries: exchanges(31, 40), hasMore: true, nextOffset: 60 },
        })
        .mockResolvedValueOnce({
          data: { entries: exchanges(26, 30), hasMore: true, nextOffset: 50 },
        })
        .mockResolvedValueOnce({
          data: { entries: exchanges(21, 25), hasMore: true, nextOffset: 40 },
        });
      render();
      await establish();
      await act(async () => {
        await result.current.loadOlderTimeline();
      });
      await flush();
      act(() => {
        result.current.syncEngineRef.current!.mutate("s1", (live) =>
          applyStreamEvent(
            live,
            evt("user_message", JSON.stringify({ text: "pending" })),
          ),
        );
      });
      await flush();
      act(() => {
        if (mode === "restart")
          result.current.syncEngineRef.current!.restartAll("reconnect");
        else result.current.reconcileSession("s1", "resend");
      });
      await flush();
      const users = result.current.timeline.items
        .filter((i) => i.kind === "message" && i.role === "user")
        .map((i) => (i.kind === "message" ? i.text : ""));
      expect(users).toEqual([
        ...Array.from({ length: 40 }, (_, i) => `user ${1 + i}`),
        "pending",
      ]);
      expect(request.mock.calls.at(-1)?.[0]).toEqual(
        expect.objectContaining({ before: 50, limit: 3 }),
      );
      expect(result.current.canLoadOlderTimeline).toBe(false);
      const calls = request.mock.calls.length;
      await act(async () => {
        expect(await result.current.loadOlderTimeline()).toBe(false);
      });
      expect(request).toHaveBeenCalledTimes(calls);
    },
  );

  describe("surface", () => {
    test("returns an empty timeline when no session is selected", () => {
      render();
      expect(result.current.timeline.items).toEqual([]);
      expect(result.current.timeline.streaming).toBe(false);
      expect(result.current.timelinePending).toBe(false);
      expect(result.current.timelineError).toBeNull();
      expect(typeof result.current.handleEvent).toBe("function");
      expect(typeof result.current.reconcileSession).toBe("function");
    });

    test("flags a pending timeline for a non-draft selected session", () => {
      options.selectedSessionId = "s1";
      render();
      expect(result.current.timelinePending).toBe(true);
    });

    test("does not flag a draft timeline as pending", () => {
      options.selectedSessionId = "s1";
      options.draft = true;
      render();
      expect(result.current.timelinePending).toBe(false);
    });
  });

  describe("reconcileSession", () => {
    test("reconciles a specific session when one is provided", () => {
      render();
      const engine = result.current.syncEngineRef.current!;
      const reconcile = jest.spyOn(engine, "reconcile");
      result.current.reconcileSession("s1", "open", "run-1");
      expect(reconcile).toHaveBeenCalledWith("s1", "open", "run-1");
    });

    test("reconciles all sessions when no session id is provided", () => {
      render();
      const engine = result.current.syncEngineRef.current!;
      const reconcileAll = jest.spyOn(engine, "reconcileAll");
      result.current.reconcileSession(undefined, "reconnect");
      expect(reconcileAll).toHaveBeenCalledWith("reconnect");
    });

    test("is a no-op before the engine exists", () => {
      render();
      result.current.syncEngineRef.current = null;
      expect(() => result.current.reconcileSession("s1", "open")).not.toThrow();
    });
  });

  describe("handleEvent", () => {
    test("background token bursts do not fetch history while status notifications stay live", async () => {
      options.selectedSessionId = "front";
      render();
      for (let index = 0; index < 10_000; index++) {
        result.current.handleEvent(
          evt("text_chunk", '{"text":"x"}', "r", index),
          `background-${index % 100}`,
        );
      }
      for (const type of [
        "agent_end",
        "approval_request",
        "approval_decision",
      ]) {
        result.current.handleEvent(evt(type, "{}", "r"), "background-1");
      }
      await flush();
      expect(request).not.toHaveBeenCalled();
      expect(options.refreshSessions).toHaveBeenCalledTimes(3);
      expect(
        result.current.syncEngineRef.current!.timelineFor("background-1"),
      ).toBeNull();
    });

    test("ignores events with an empty session id", () => {
      render();
      result.current.handleEvent(evt("agent_start", "{}"), "");
      expect(request).not.toHaveBeenCalled();
    });

    test.each(["provider_config_changed", "model_visibility_changed"])("%s refreshes models without reconciling a global timeline", type => {
      render();
      const reconcile = jest.spyOn(result.current.syncEngineRef.current!, "reconcile");
      result.current.handleEvent(evt(type, "{}"), "_global");
      expect(options.refreshModels).toHaveBeenCalledTimes(1);
      expect(reconcile).not.toHaveBeenCalled();
      expect(request).not.toHaveBeenCalled();
    });

    test("run_snapshot reconciles the session as resend", () => {
      render();
      const engine = result.current.syncEngineRef.current!;
      const reconcile = jest.spyOn(engine, "reconcile");
      result.current.handleEvent(evt("run_snapshot", "{}", "run-1"), "s1");
      expect(reconcile).toHaveBeenCalledWith("s1", "resend", "run-1");
    });

    test("session_name_changed updates the title override and refreshes sessions", () => {
      render();
      result.current.handleEvent(
        evt("session_name_changed", JSON.stringify({ name: " Renamed " })),
        "s1",
      );
      const updater = (options.setTitleOverrides as jest.Mock).mock
        .calls[0][0] as (
        prev: Record<string, string>,
      ) => Record<string, string>;
      expect(updater({})).toEqual({ s1: "Renamed" });
      expect(options.refreshSessions).toHaveBeenCalled();
    });

    test("session_name_changed ignores a blank name", () => {
      render();
      result.current.handleEvent(
        evt("session_name_changed", JSON.stringify({ name: "  " })),
        "s1",
      );
      expect(options.setTitleOverrides).not.toHaveBeenCalled();
      expect(options.refreshSessions).not.toHaveBeenCalled();
    });

    test("session_name_changed swallows malformed JSON", () => {
      render();
      result.current.handleEvent(evt("session_name_changed", "not json"), "s1");
      expect(options.setTitleOverrides).not.toHaveBeenCalled();
    });

    test("user_message hydrates attachments for the session", () => {
      options.selectedSessionId = "s1";
      render();
      const hydrate = jest.fn(async () => {});
      result.current.hydrateAttachmentsRef.current = hydrate;
      result.current.handleEvent(
        evt("user_message", JSON.stringify({ text: "hi" })),
        "s1",
      );
      expect(hydrate).toHaveBeenCalledWith("s1");
    });

    test("approval_decision mutates a matching approval item", async () => {
      options.selectedSessionId = "s1";
      render();
      const engine = result.current.syncEngineRef.current!;
      engine.mutate("s1", (tl) => ({
        ...tl,
        items: [
          ...tl.items,
          {
            id: "approval:a1",
            kind: "approval",
            payload: { approval_request_id: "a1", tool_name: "bash" },
          },
        ],
      }));
      await flush();
      result.current.handleEvent(
        evt(
          "approval_decision",
          JSON.stringify({ approval_request_id: "a1", status: "approved" }),
        ),
        "s1",
      );
      await flush();
      const approval = result.current.timeline.items.find(
        (i) => i.kind === "approval",
      );
      expect(approval).toMatchObject({ decision: "approved" });
    });

    test("approval_decision leaves an unmatched approval item alone", async () => {
      options.selectedSessionId = "s1";
      render();
      const engine = result.current.syncEngineRef.current!;
      engine.mutate("s1", (tl) => ({
        ...tl,
        items: [
          ...tl.items,
          {
            id: "approval:a1",
            kind: "approval",
            payload: { approval_request_id: "a1", tool_name: "bash" },
          },
        ],
      }));
      await flush();
      result.current.handleEvent(
        evt(
          "approval_decision",
          JSON.stringify({ approval_request_id: "nope", status: "rejected" }),
        ),
        "s1",
      );
      await flush();
      const approval = result.current.timeline.items.find(
        (i) => i.kind === "approval",
      );
      expect(approval?.decision).toBeUndefined();
    });

    test("approval_decision ignores an invalid status", () => {
      render();
      const engine = result.current.syncEngineRef.current!;
      const mutate = jest.spyOn(engine, "mutate");
      result.current.handleEvent(
        evt(
          "approval_decision",
          JSON.stringify({ approval_request_id: "a1", status: "pending" }),
        ),
        "s1",
      );
      expect(mutate).not.toHaveBeenCalled();
    });

    test("approval_decision swallows malformed JSON", () => {
      render();
      result.current.handleEvent(evt("approval_decision", "not json"), "s1");
      expect(request).not.toHaveBeenCalled();
    });

    test("agent_end refreshes the session list", () => {
      render();
      result.current.handleEvent(evt("agent_end", "{}"), "s1");
      expect(options.refreshSessions).toHaveBeenCalled();
    });
  });

  describe("loadHistory", () => {
    test("a byte-trimmed gap page re-bridges in place without losing history", async () => {
      jest.useFakeTimers();
      try {
      // The desktop bridge sheds whole oldest exchanges to fit its reply
      // budget, advancing nextOffset past the dropped rows and setting
      // hasMore. A refresh hitting that shape must re-read the range
      // untrimmed and bridge the gap in the same pass — never fail the
      // refresh and strand the visible older history.
      options.selectedSessionId = "s1";
      options.selectedRef.current = "s1";
      // One ordinal per entry, so a page [start, before) is flush by
      // construction (start + entries.length === before).
      const journal: ReturnType<typeof userEntry>[] = [];
      const push = (entry: ReturnType<typeof userEntry>) => {
        journal.push(entry);
      };
      for (const [q, a, run] of [
        ["earlier question", "earlier answer", "run-old"],
        ["middle question", "middle answer", "run-mid"],
        ["gap question", "gap answer", "run-gap"],
        ["latest question", "latest answer", "run-new"],
      ] as const) {
        push(userEntry(`u-${run}`, q));
        push(assistantEntry(`a-${run}`, a, run));
      }
      const page = (before: number, trim: boolean) => {
        // The bridge pages by user exchanges (limit 3), not raw rows.
        let start = before;
        let users = 0;
        while (start > 0 && users < 3) {
          start -= 1;
          if (journal[start]?.role === "user") users += 1;
        }
        const slice = journal.slice(start, before);
        // Simulate the byte budget: the newest page always pays it, and a
        // mid-history page pays it once it is "large" (all three exchanges,
        // like the post-send refresh gap).
        const paysBudget = trim && users > 1 &&
          (before === journal.length || (before === 6 && users >= 3));
        if (!paysBudget) {
          return {
            entries: slice,
            hasMore: start > 0,
            nextOffset: start,
          };
        }
        // The byte-budget trim: shed whole oldest exchanges, keep the newest
        // user exchange, advance the cursor, always hasMore.
        const newestUser = slice.reduce(
          (acc, entry, index) => (entry.role === "user" ? index : acc),
          0,
        );
        const kept = slice.slice(newestUser);
        return {
          entries: kept,
          hasMore: true,
          nextOffset: before - kept.length,
        };
      };
      let entryCall = 0;
      request.mockImplementation(async (command: {
        type: string;
        before?: number;
        untrimmed?: boolean;
      }) => {
        if (command.type !== "get_session_entries") return { data: {} };
        entryCall += 1;
        const before = Math.min(command.before ?? journal.length, journal.length);
        // The first two entry reads (the open tail and the explicit
        // loadOlder pull) answer complete pages; from the post-send refresh
        // on, the bridge's byte budget trims backward pages unless the
        // reader opts out.
        const trim = entryCall > 2 && command.untrimmed !== true;
        const result = page(before, trim);
        return { data: result };
      });
      render();
      await establish();
      await act(async () => {
        await result.current.loadOlderTimeline();
      });
      await flush();
      const initial = result.current.timeline.items
        .filter((i) => i.kind === "message")
        .map((i) => (i.kind === "message" ? i.text : ""));
      expect(initial[0]).toBe("earlier question");
      expect(initial).toContain("latest question");

      // The refresh (e.g. after sending) hits the trimmed gap page, re-reads
      // it untrimmed, and bridges the hole in the same pass — the committed
      // conversation grows in place, never collapsing to the tail. Simulate
      // the appended exchange that makes the refresh's tail move past the
      // paged window.
      push(userEntry("u-run-new2", "follow-up question"));
      push(assistantEntry("a-run-new2", "follow-up answer", "run-new2"));
      act(() => result.current.reconcileSession("s1", "resend"));
      await flush();
      expect(
        result.current.timeline.items
          .filter((i) => i.kind === "message")
          .map((i) => (i.kind === "message" ? i.text : "")),
      ).toEqual([
        "earlier question",
        "earlier answer",
        "middle question",
        "middle answer",
        "gap question",
        "gap answer",
        "latest question",
        "latest answer",
        "follow-up question",
        "follow-up answer",
      ]);
      expect(result.current.timelineSyncStatus).toBe("idle");
      } finally {
        act(() => renderer?.unmount());
        renderer = null;
        jest.useRealTimers();
      }
    });

    test("a refresh gap wider than one page still recovers when every page is byte-trimmed", async () => {
      jest.useFakeTimers();
      try {
      // Regression: a tool-dense session whose single exchange exceeds the
      // bridge's 512KB page budget. The post-send refresh's tail page is
      // trimmed to the newest exchange, and the gap back to the paged window
      // spans several more pages that are EACH also over budget. Filling that
      // gap must not throw `history_gap_cursor_invalid` and strand the
      // visible conversation on the trimmed tail — every older exchange must
      // survive. (Session 20260925-171247: one exchange ~531KB, tail trimmed
      // 1.1MB → 161KB.)
      options.selectedSessionId = "s1";
      options.selectedRef.current = "s1";
      const PAGE_BUDGET = 512 * 1024;
      // Build a journal of exchanges; each exchange = 1 user + many
      // assistant/tool rows, sized so a 3-exchange page exceeds the budget.
      type Row = ReturnType<typeof userEntry> & { bytes: number };
      const journal: Row[] = [];
      const pushRow = (entry: ReturnType<typeof userEntry>, bytes: number) => {
        journal.push(Object.assign(entry, { bytes }));
      };
      // 6 older exchanges, then the 3 in the tail window. Each exchange's
      // rows carry enough bytes that [3 exchanges] > 512KB.
      const exchangeSizes = [
        200_000, 24_000, 48_000, 47_000, 532_000, 496_000, 160_000,
      ];
      exchangeSizes.forEach((total, ex) => {
        const run = `run-${ex}`;
        pushRow(userEntry(`u-${run}`, `question ${ex}`), 100);
        // Split the exchange body across many assistant rows (tools in the
        // real session). Keep every row small; the SUM is what trips the budget.
        const rows = Math.max(1, Math.round(total / 8_000));
        const perRow = Math.floor((total - 100) / rows);
        for (let i = 0; i < rows; i += 1) {
          pushRow(assistantEntry(`a-${run}-${i}`, `answer ${ex}.${i}`, run), perRow);
        }
      });
      // Bridge page: walk back 3 user exchanges, then apply the byte budget
      // (shed whole oldest exchanges until serialized size fits), unless the
      // reader opts out with `untrimmed`.
      const bridgePage = (before: number, untrimmed: boolean) => {
        let start = before;
        let users = 0;
        while (start > 0 && users < 3) {
          start -= 1;
          if (journal[start]?.role === "user") users += 1;
        }
        let slice = journal.slice(start, before);
        let removed = 0;
        if (!untrimmed) {
          const size = () => slice.reduce((n, r) => n + r.bytes, 0);
          while (size() > PAGE_BUDGET) {
            const nextUser = slice.findIndex(
              (row, index) => index > 0 && row.role === "user",
            );
            if (nextUser < 0) break;
            removed += nextUser;
            slice = slice.slice(nextUser);
          }
        }
        return {
          entries: slice.map(({ bytes: _b, ...entry }) => entry),
          hasMore: start + removed > 0,
          nextOffset: start + removed,
          trimmed: removed > 0,
        };
      };
      request.mockImplementation(async (command: {
        type: string;
        before?: number;
        untrimmed?: boolean;
      }) => {
        if (command.type !== "get_session_entries") return { data: {} };
        const before = Math.min(command.before ?? journal.length, journal.length);
        return { data: bridgePage(before, command.untrimmed === true) };
      });
      render();
      await establish();
      // Cold open: NO older paging. retained is just the (trimmed) tail. Send
      // a new large exchange: the refresh's tail is byte-trimmed to start
      // beyond the previous tail's cursor, opening a gap the fill must bridge.
      pushRow(userEntry("u-run-new", "new question"), 100);
      for (let i = 0; i < 70; i += 1)
        pushRow(assistantEntry(`a-run-new-${i}`, `new answer ${i}`, "run-new"), 8_000);
      act(() => result.current.reconcileSession("s1", "resend"));
      await flush();
      const healed = result.current.timeline.items
        .filter((i) => i.kind === "message")
        .map((i) => (i.kind === "message" ? i.text : ""));
      // The pre-send tail exchanges (5, 6) must survive the refresh — the
      // refresh must not collapse the visible window to just the new run.
      expect(healed).toContain("question 5");
      expect(healed).toContain("question 6");
      expect(healed).toContain("new question");
      expect(result.current.timelineSyncStatus).toBe("idle");
      } finally {
        act(() => renderer?.unmount());
        renderer = null;
        jest.useRealTimers();
      }
    });

    test("returns an empty timeline when the client is absent", async () => {
      render();
      const engine = result.current.syncEngineRef.current!;
      engine.mutate("s1", () => emptyTimeline());
      await flush();
      options.clientRef.current = null;
      await act(async () => {
        await result.current.hydrateAttachmentsRef.current("s1");
      });
      expect(result.current.syncEngineRef.current).toBeTruthy();
    });

    test("hydrate is a no-op when the session has no timeline", async () => {
      render();
      await act(async () => {
        await result.current.hydrateAttachmentsRef.current("s1");
      });
      expect(request).not.toHaveBeenCalled();
    });

    test("hydrate merges durable attachments and swallows history failures", async () => {
      options.selectedSessionId = "s1";
      render();
      const engine = result.current.syncEngineRef.current!;
      engine.mutate("s1", () => emptyTimeline());
      await flush();
      request.mockRejectedValue(new Error("no history"));
      await act(async () => {
        await result.current.hydrateAttachmentsRef.current("s1");
      });
      expect(request).toHaveBeenCalled();
    });

    test("loads only the latest backward page", async () => {
      options.selectedSessionId = "s1";
      options.selectedRef.current = "s1";
      request
        .mockResolvedValueOnce({ success: true, data: {} }) // get_state (no active run)
        .mockResolvedValueOnce({
          success: true,
          data: {
            entries: [
              userEntry("e3", "latest"),
              assistantEntry("e4", "answer"),
            ],
            hasMore: true,
            nextOffset: 20,
          },
        });
      render();
      await establish();
      const texts = result.current.timeline.items
        .filter((i) => i.kind === "message")
        .map((i) => (i.kind === "message" ? i.text : ""));
      expect(texts).toEqual(["latest", "answer"]);
      expect(request).toHaveBeenCalledTimes(2);
      expect(request.mock.calls[1]?.[0]).toEqual(
        expect.objectContaining({
          type: "get_session_entries",
          before: Number.MAX_SAFE_INTEGER,
          limit: 3,
        }),
      );
      expect(result.current.canLoadOlderTimeline).toBe(true);
    });

    test("three-exchange pages reach all older history without gaps or duplicates", async () => {
      options.selectedSessionId = "s1";
      const entries = Array.from(
        { length: 8 },
        (_, index) => index + 1,
      ).flatMap((n) => [
        userEntry(`u${n}`, `user ${n}`),
        assistantEntry(`a${n}`, `answer ${n}`),
      ]);
      request.mockImplementation(
        async (command: { type: string; before: number; limit: number }) => {
          if (command.type === "get_state") return { data: {} };
          expect(command.type).toBe("get_session_entries");
          expect(command.limit).toBe(3);
          const end = Math.min(command.before, entries.length);
          const start = Math.max(0, end - command.limit * 2);
          return {
            data: {
              entries: entries.slice(start, end),
              hasMore: start > 0,
              nextOffset: start,
            },
          };
        },
      );
      render();
      await establish();
      expect(result.current.timeline.items).toHaveLength(6);
      expect(result.current.canLoadOlderTimeline).toBe(true);

      await act(async () => {
        await result.current.loadOlderTimeline();
      });
      await flush();
      expect(result.current.timeline.items).toHaveLength(12);
      expect(result.current.canLoadOlderTimeline).toBe(true);

      await act(async () => {
        await result.current.loadOlderTimeline();
      });
      await flush();
      expect(result.current.timeline.items.map((item) => item.id)).toEqual(
        entries.map((entry) => `m_${entry.id}`),
      );
      expect(result.current.canLoadOlderTimeline).toBe(false);
      expect(
        request.mock.calls
          .filter(([command]) => command.type === "get_session_entries")
          .map(([command]) => command.before),
      ).toEqual([Number.MAX_SAFE_INTEGER, 10, 4]);
      const requests = request.mock.calls.length;
      await act(async () => {
        expect(await result.current.loadOlderTimeline()).toBe(false);
      });
      expect(request).toHaveBeenCalledTimes(requests);
    });

    test("loads one older page and prepends it without refetching the tail", async () => {
      options.selectedSessionId = "s1";
      options.selectedRef.current = "s1";
      request
        .mockResolvedValueOnce({ success: true, data: {} })
        .mockResolvedValueOnce({
          success: true,
          data: {
            entries: [
              userEntry("e3", "latest"),
              assistantEntry("e4", "answer"),
            ],
            hasMore: true,
            nextOffset: 20,
          },
        })
        .mockResolvedValueOnce({
          success: true,
          data: {
            entries: [
              userEntry("e1", "older"),
              assistantEntry("e2", "older answer"),
            ],
            hasMore: false,
            nextOffset: 0,
          },
        })
        .mockResolvedValueOnce({ success: true, data: {} })
        .mockResolvedValueOnce({
          success: true,
          data: {
            entries: [
              userEntry("e3", "latest"),
              assistantEntry("e4", "reconciled answer"),
            ],
            hasMore: true,
            nextOffset: 20,
          },
        });
      render();
      await establish();
      await act(async () => {
        await result.current.loadOlderTimeline();
      });
      await flush();
      const texts = result.current.timeline.items
        .filter((i) => i.kind === "message")
        .map((i) => (i.kind === "message" ? i.text : ""));
      expect(texts).toEqual(["older", "older answer", "latest", "answer"]);
      expect(request.mock.calls[2]?.[0]).toEqual(
        expect.objectContaining({
          type: "get_session_entries",
          before: 20,
          limit: 3,
        }),
      );
      expect(result.current.canLoadOlderTimeline).toBe(false);

      act(() => result.current.reconcileSession("s1", "resend"));
      await flush();
      const reconciledTexts = result.current.timeline.items
        .filter((i) => i.kind === "message")
        .map((i) => (i.kind === "message" ? i.text : ""));
      expect(reconciledTexts).toEqual([
        "older",
        "older answer",
        "latest",
        "reconciled answer",
      ]);
      expect(request.mock.calls[4]?.[0]).toEqual(
        expect.objectContaining({ before: Number.MAX_SAFE_INTEGER, limit: 3 }),
      );
    });

    test.each([false, true])(
      "preserves a completed live reply when its entry cursor lagged during streaming (hydrate=%s)",
      async (hydrate) => {
        options.selectedSessionId = "s1";
        const thirdUser = { ...userEntry("u3", "long task"), runId: "r3" };
        const steps = Array.from({ length: 145 }, (_, n) => `step ${n}`);
        const longReply = steps.join("\n\n");
        request
          .mockResolvedValueOnce({ data: {} })
          .mockResolvedValueOnce({ data: {
            entries: [userEntry("u1", "first"), assistantEntry("a1", "first reply"),
              userEntry("u2", "second"), assistantEntry("a2", "second reply"), thirdUser],
            hasMore: true, nextOffset: 7,
          } });
        render();
        await establish(); // The durable window ends at 12, before the long run.
        const engine = result.current.syncEngineRef.current!;
        act(() => engine.mutate("s1", live => applyStreamEvents(live, [
          evt("agent_start", "{}", "r3", 0),
          evt("text_chunk", JSON.stringify({ text: longReply }), "r3", 1),
          evt("agent_end", '{"state":"completed","duration_ms":2997094}', "r3", 2),
        ])));
        await flush();
        expect(result.current.timeline.items.at(-1)).toMatchObject({ text: longReply, streaming: false });
        expect(request).toHaveBeenCalledTimes(2); // Live output did not advance the history cursor.
        act(() => engine.mutate("s1", live => commitAcknowledgedUserMessage(live, {
          id: "local:follow-up", runId: "r4", text: "follow-up",
        })));
        await flush();
        const tail = { data: {
          entries: [{ ...userEntry("u4", "follow-up"), runId: "r4" }, assistantEntry("a4", "new reply", "r4")],
          hasMore: true, nextOffset: 157,
        } };
        if (hydrate) {
          request.mockResolvedValueOnce(tail);
          await act(async () => { await result.current.hydrateAttachmentsRef.current("s1"); });
        }
        let resolveGap!: (value: unknown) => void;
        request
          .mockResolvedValueOnce({ data: {} })
          .mockResolvedValueOnce(tail)
          .mockImplementationOnce(() => new Promise(resolve => { resolveGap = resolve; }));
        act(() => result.current.reconcileSession("s1", "resend"));
        await flush();
        // A capped newest page is not a replacement for the displayed conversation.
        expect(result.current.timeline.items.some(item => item.id === "m_u1")).toBe(true);
        expect(result.current.timeline.items.some(item => item.kind === "message" && item.text === longReply)).toBe(true);
        expect(request.mock.calls.at(-1)?.[0]).toEqual(expect.objectContaining({ before: 157 }));
        await act(async () => resolveGap({ data: {
          entries: [thirdUser, ...steps.map((text, n) => assistantEntry(`step-${n}`, text, "r3"))],
          hasMore: true, nextOffset: 11,
        } }));
        await flush();
        expect(result.current.timeline.items.map(item => item.kind === "message" ? item.text : "")).toEqual([
          "first", "first reply", "second", "second reply", "long task", longReply, "follow-up", "new reply",
        ]);
        request.mockResolvedValueOnce({ data: {
          entries: [userEntry("u0", "older")], hasMore: false, nextOffset: 0,
        } });
        await act(async () => { await result.current.loadOlderTimeline(); });
        expect(request.mock.calls.at(-1)?.[0]).toEqual(expect.objectContaining({ before: 7 }));
        expect(result.current.canLoadOlderTimeline).toBe(false);
      },
    );

    test.each(["offline", "nonadvancing", "incomplete"])(
      "a %s gap page preserves the committed history and cursor until retry succeeds",
      async (failure) => {
        jest.useFakeTimers();
        try {
          options.selectedSessionId = "s1";
          const original = [userEntry("u1", "first"), assistantEntry("a1", "first reply")];
          const tail = { data: {
            entries: [userEntry("u3", "latest"), assistantEntry("a3", "latest reply")],
            nextOffset: 24, hasMore: true,
          } };
          request.mockResolvedValueOnce({ data: {} }).mockResolvedValueOnce({ data: {
            entries: original, nextOffset: 20, hasMore: true,
          } });
          render();
          await establish();
          request.mockResolvedValueOnce({ data: {} }).mockResolvedValueOnce(tail);
          if (failure === "offline") request.mockRejectedValueOnce(new Error("offline"));
          else request.mockResolvedValueOnce({ data: {
            entries: [userEntry("u2", "middle")],
            nextOffset: failure === "nonadvancing" ? 24 : 22, hasMore: true,
          } });
          act(() => result.current.reconcileSession("s1", "resend"));
          await flush();
          expect(result.current.timeline.items.map(item => item.id)).toEqual(["m_u1", "m_a1"]);
          expect(result.current.timelineSyncStatus).toBe("retrying");
          request.mockResolvedValueOnce({ data: {} }).mockResolvedValueOnce(tail)
            .mockResolvedValueOnce({ data: {
              entries: [userEntry("u2", "middle"), assistantEntry("a2", "middle reply")],
              nextOffset: 22, hasMore: true,
            } });
          act(() => result.current.syncEngineRef.current!.restart("s1", "reconnect"));
          await flush();
          expect(result.current.timeline.items.map(item => item.id)).toEqual([
            "m_u1", "m_a1", "m_u2", "m_a2", "m_u3", "m_a3",
          ]);
          request.mockResolvedValueOnce({ data: {
            entries: [userEntry("u0", "older")], nextOffset: 0, hasMore: false,
          } });
          await act(async () => { await result.current.loadOlderTimeline(); });
          expect(request.mock.calls.at(-1)?.[0]).toEqual(expect.objectContaining({ before: 20 }));
        } finally {
          act(() => renderer?.unmount());
          renderer = null;
          jest.useRealTimers();
        }
      },
    );

    test("a failed replay cannot commit a downloaded history cursor without its rows", async () => {
      jest.useFakeTimers();
      try {
        options.selectedSessionId = "s1";
        request.mockResolvedValueOnce({ data: {} }).mockResolvedValueOnce({ data: {
          entries: [userEntry("u1", "first")], nextOffset: 20, hasMore: true,
        } });
        render();
        await establish();
        const tail = { data: {
          entries: [{ ...userEntry("u3", "latest"), runId: "r3" }], nextOffset: 23, hasMore: true,
        } };
        const gap = { data: {
          entries: [userEntry("u2", "middle"), assistantEntry("a2", "middle reply")], nextOffset: 21, hasMore: true,
        } };
        request.mockResolvedValueOnce({ data: { activeRun: { runId: "r3" } } })
          .mockResolvedValueOnce(tail).mockResolvedValueOnce(gap)
          .mockRejectedValueOnce(new Error("replay offline"));
        act(() => result.current.reconcileSession("s1", "resend"));
        await flush();
        expect(result.current.timeline.items.map(item => item.id)).toEqual(["m_u1"]);
        expect(result.current.timelineSyncStatus).toBe("retrying");
        request.mockResolvedValueOnce({ data: {} }).mockResolvedValueOnce(tail).mockResolvedValueOnce(gap);
        act(() => result.current.syncEngineRef.current!.restart("s1", "reconnect"));
        await flush();
        expect(request.mock.calls.filter(([command]) => command.before === 23)).toHaveLength(2);
        expect(result.current.timeline.items.map(item => item.id)).toEqual(["m_u1", "m_u2", "m_a2", "m_u3"]);
        request.mockResolvedValueOnce({ data: {
          entries: [userEntry("u0", "older")], nextOffset: 0, hasMore: false,
        } });
        await act(async () => { await result.current.loadOlderTimeline(); });
        expect(request.mock.calls.at(-1)?.[0]).toEqual(expect.objectContaining({ before: 20 }));
      } finally {
        act(() => renderer?.unmount());
        renderer = null;
        jest.useRealTimers();
      }
    });

    test("a superseded gap read cannot overwrite a restarted lane's paging state", async () => {
      options.selectedSessionId = "s1";
      request.mockResolvedValueOnce({ data: {} }).mockResolvedValueOnce({ data: {
        entries: [userEntry("u1", "first")], nextOffset: 20, hasMore: true,
      } });
      render();
      await establish();
      let finishOld!: (value: unknown) => void;
      request.mockResolvedValueOnce({ data: {} }).mockResolvedValueOnce({ data: {
        entries: [userEntry("u3", "tail")], nextOffset: 24, hasMore: true,
      } }).mockImplementationOnce(() => new Promise(resolve => { finishOld = resolve; }));
      act(() => result.current.reconcileSession("s1", "resend"));
      await flush();
      expect(request.mock.calls.at(-1)?.[0]).toEqual(expect.objectContaining({ before: 24 }));
      request.mockResolvedValueOnce({ data: {} }).mockResolvedValueOnce({ data: {
        entries: [userEntry("fresh", "fresh history")], nextOffset: 0, hasMore: false,
      } });
      act(() => result.current.syncEngineRef.current!.restart("s1", "reconnect"));
      await flush();
      expect(result.current.canLoadOlderTimeline).toBe(false);
      await act(async () => finishOld({ data: {
        entries: [userEntry("old2", "obsolete"), assistantEntry("old3", "obsolete reply"), userEntry("old4", "obsolete next")],
        nextOffset: 21, hasMore: true,
      } }));
      await flush();
      expect(result.current.timeline.items.map(item => item.id)).toEqual(["m_fresh"]);
      expect(result.current.canLoadOlderTimeline).toBe(false);
    });

    test("explicitly reopening a short cached window does not backfill a remote gap", async () => {
      options.selectedSessionId = "s1";
      request.mockResolvedValueOnce({ data: {} }).mockResolvedValueOnce({ data: {
        entries: [userEntry("u1", "old")], nextOffset: 20, hasMore: true,
      } });
      render();
      await establish();
      act(() => result.current.prepareTimelineOpen("s1"));
      request.mockResolvedValueOnce({ data: {} }).mockResolvedValueOnce({ data: {
        entries: [userEntry("u100", "latest")], nextOffset: 100, hasMore: true,
      } });
      await act(async () => { await result.current.syncEngineRef.current!.open("s1"); });
      await flush();
      expect(request).toHaveBeenCalledTimes(4);
      expect(result.current.timeline.items.map(item => item.id)).toEqual(["m_u100"]);
      expect(result.current.canLoadOlderTimeline).toBe(true);
    });

    test.each([
      { acknowledged: false, hasOlder: false },
      { acknowledged: true, hasOlder: false },
      { acknowledged: false, hasOlder: true },
      { acknowledged: true, hasOlder: true },
    ])(
      "sending after a long reply retains an adjacent capped page ($acknowledged, older=$hasOlder)",
      async ({ acknowledged, hasOlder }) => {
        options.selectedSessionId = "s1";
        const longReply = "long reply ".repeat(1000);
        const initialOffset = hasOlder ? 20 : 0;
        // The bridge may omit the entire large exchange from the next tail
        // page. Its cursor proves adjacency even though no entry ids overlap.
        const previousEntries = [
          { ...userEntry("u1", "previous prompt"), runId: "r1" },
          assistantEntry("intermediate", "intermediate output", "r1"),
          assistantEntry("a1", longReply, "r1"),
        ];
        request
          .mockResolvedValueOnce({ data: {} })
          .mockResolvedValueOnce({
            data: { entries: previousEntries, hasMore: hasOlder, nextOffset: initialOffset },
          });
        render();
        await establish();
        if (acknowledged) {
          act(() => result.current.syncEngineRef.current!.mutate("s1", (live) =>
            commitAcknowledgedUserMessage(live, {
              id: "local:follow-up", runId: "r2", text: "follow-up",
            }),
          ));
          await flush();
        }
        request
          .mockResolvedValueOnce({ data: {} })
          .mockResolvedValueOnce({
            data: {
              entries: [
                { ...userEntry("u2", "follow-up"), runId: "r2" },
                assistantEntry("a2", "new reply", "r2"),
              ],
              hasMore: true,
              nextOffset: initialOffset + previousEntries.length,
            },
          });
        act(() => result.current.reconcileSession("s1", "resend"));
        await flush();
        expect(result.current.timeline.items.map(item => item.id)).toEqual([
          "m_u1", "m_a1", "m_u2", "m_a2",
        ]);
        expect(result.current.timeline.items[1]).toMatchObject({
          text: `intermediate output\n\n${longReply}`,
        });
        expect(result.current.canLoadOlderTimeline).toBe(hasOlder);
        if (!hasOlder) return;

        request.mockResolvedValueOnce({
          data: {
            entries: [userEntry("u0", "older prompt"), assistantEntry("a0", "older reply")],
            hasMore: false,
            nextOffset: 0,
          },
        });
        await act(async () => { await result.current.loadOlderTimeline(); });
        await flush();
        expect(request.mock.calls.at(-1)?.[0]).toEqual(
          expect.objectContaining({ before: 20 }),
        );
        expect(result.current.timeline.items.map(item => item.id)).toEqual([
          "m_u0", "m_a0", "m_u1", "m_a1", "m_u2", "m_a2",
        ]);
        expect(result.current.canLoadOlderTimeline).toBe(false);
      },
    );

    test("reopening a warm timeline renders only the latest three exchanges", async () => {
      options.selectedSessionId = "s1";
      options.selectedRef.current = "s1";
      const exchanges = (start: number, end: number) =>
        Array.from({ length: end - start + 1 }, (_, i) => start + i).flatMap(
          (n) => [
            userEntry(`u${n}`, `user ${n}`),
            assistantEntry(`a${n}`, `answer ${n}`),
          ],
        );
      request
        .mockResolvedValueOnce({ success: true, data: {} })
        .mockResolvedValueOnce({
          success: true,
          data: { entries: exchanges(11, 20), hasMore: true, nextOffset: 20 },
        })
        .mockResolvedValueOnce({
          success: true,
          data: { entries: exchanges(1, 10), hasMore: false, nextOffset: 0 },
        });
      render();
      await establish();
      await act(async () => {
        await result.current.loadOlderTimeline();
      });
      await flush();
      expect(result.current.timeline.items).toHaveLength(40);

      act(() => result.current.prepareTimelineOpen("s1"));
      await flush();
      // Cache warmth must not mount all previously paged tool-heavy turns while
      // waiting for the new network request.
      expect(result.current.timeline.items).toHaveLength(6);
      expect(result.current.timeline.items[0]?.id).toBe("m_u18");
      request.mockResolvedValueOnce({ data: {} }).mockResolvedValueOnce({
        data: { entries: exchanges(18, 20), hasMore: true, nextOffset: 34 },
      });
      await act(async () => {
        await result.current.syncEngineRef.current!.open("s1");
      });
      await flush();

      expect(
        result.current.timeline.items
          .filter((item) => item.kind === "message" && item.role === "user")
          .map((item) => (item.kind === "message" ? item.text : "")),
      ).toEqual(["user 18", "user 19", "user 20"]);
      expect(result.current.canLoadOlderTimeline).toBe(true);
    });

    test("a downloaded page stays busy until the sync lane publishes it", async () => {
      options.selectedSessionId = "s1";
      let resolveReplay!: (value: unknown) => void;
      request
        .mockResolvedValueOnce({ data: { activeRun: { runId: "run" } } })
        .mockResolvedValueOnce({
          data: {
            entries: [userEntry("latest", "latest")],
            hasMore: true,
            nextOffset: 20,
          },
        })
        .mockImplementationOnce(
          () =>
            new Promise((resolve) => {
              resolveReplay = resolve;
            }),
        )
        .mockResolvedValueOnce({
          data: {
            entries: [userEntry("older", "older")],
            hasMore: false,
            nextOffset: 0,
          },
        });
      render();
      await establish();
      let page!: ReturnType<Result["loadOlderTimeline"]>;
      act(() => {
        page = result.current.loadOlderTimeline();
      });
      await flush();
      expect(result.current.loadingOlderTimeline).toBe(true);
      expect(result.current.canLoadOlderTimeline).toBe(true);
      expect(
        result.current.timeline.items.map((item) => item.id),
      ).not.toContain("m_older");
      await act(async () => {
        expect(await result.current.loadOlderTimeline()).toBe(false);
      });
      expect(request).toHaveBeenCalledTimes(4);
      await act(async () => {
        resolveReplay({ data: { events: [], hasMore: false } });
        expect(await page).toContain("m_older");
      });
      expect(result.current.timeline.items.map((item) => item.id)).toContain(
        "m_older",
      );
      expect(result.current.loadingOlderTimeline).toBe(false);
      expect(result.current.canLoadOlderTimeline).toBe(false);
    });

    test.each(["transport", "commit"])(
      "a stalled %s cancels without advancing the cursor or installing late data",
      async (stage) => {
        jest.useFakeTimers();
        const errorSpy = jest
          .spyOn(console, "error")
          .mockImplementation(() => {});
        try {
          options.selectedSessionId = "s1";
          let release!: (value: unknown) => void;
          request
            .mockResolvedValueOnce({
              data: stage === "commit" ? { activeRun: { runId: "run" } } : {},
            })
            .mockResolvedValueOnce({
              data: {
                entries: [userEntry("latest", "latest")],
                hasMore: true,
                nextOffset: 20,
              },
            });
          if (stage === "commit") {
            request.mockImplementationOnce(
              () =>
                new Promise((resolve) => {
                  release = resolve;
                }),
            );
            request.mockResolvedValueOnce({
              data: {
                entries: [userEntry("stale", "stale")],
                hasMore: false,
                nextOffset: 0,
              },
            });
          } else {
            request.mockImplementationOnce(
              () =>
                new Promise((resolve) => {
                  release = resolve;
                }),
            );
          }
          render();
          await establish();
          let page!: ReturnType<Result["loadOlderTimeline"]>;
          act(() => {
            page = result.current.loadOlderTimeline();
          });
          await flush();
          await act(async () => {
            jest.advanceTimersByTime(30_000);
            expect(await page).toBe(false);
          });
          expect(result.current.loadingOlderTimeline).toBe(false);
          expect(result.current.canLoadOlderTimeline).toBe(true);
          release(
            stage === "commit"
              ? { data: { events: [], hasMore: false } }
              : {
                  data: {
                    entries: [userEntry("stale", "stale")],
                    hasMore: false,
                    nextOffset: 0,
                  },
                },
          );
          await flush();
          expect(
            result.current.timeline.items.map((item) => item.id),
          ).not.toContain("m_stale");
          request.mockResolvedValueOnce({
            data: {
              entries: [userEntry("retry", "retry")],
              hasMore: false,
              nextOffset: 0,
            },
          });
          await act(async () => {
            expect(await result.current.loadOlderTimeline()).toContain(
              "m_retry",
            );
          });
          const pages = request.mock.calls.filter(
            ([command]) => command.type === "get_session_entries",
          );
          expect(pages.map(([command]) => command.before)).toEqual([
            Number.MAX_SAFE_INTEGER,
            20,
            20,
          ]);
          expect(result.current.timeline.items.map((item) => item.id)).toEqual([
            "m_retry",
            "m_latest",
          ]);
        } finally {
          errorSpy.mockRestore();
          jest.useRealTimers();
        }
      },
    );

    test("switching sessions cancels an in-flight page immediately", async () => {
      const errorSpy = jest
        .spyOn(console, "error")
        .mockImplementation(() => {});
      try {
        options.selectedSessionId = "s1";
        request
          .mockResolvedValueOnce({ data: {} })
          .mockResolvedValueOnce({
            data: {
              entries: [userEntry("latest", "latest")],
              hasMore: true,
              nextOffset: 20,
            },
          })
          .mockImplementationOnce(() => new Promise(() => {}));
        render();
        await establish();
        let page!: ReturnType<Result["loadOlderTimeline"]>;
        act(() => {
          page = result.current.loadOlderTimeline();
        });
        await flush();
        options.selectedSessionId = "s2";
        options.selectedRef.current = "s2";
        await act(async () => {
          renderer!.update(createElement(Harness));
        });
        await expect(page).resolves.toBe(false);
        expect(result.current.loadingOlderTimeline).toBe(false);
      } finally {
        errorSpy.mockRestore();
      }
    });

    test("timeout retains the cursor and retry returns the committed page identities", async () => {
      const errorSpy = jest
        .spyOn(console, "error")
        .mockImplementation(() => {});
      options.selectedSessionId = "s1";
      options.selectedRef.current = "s1";
      request
        .mockResolvedValueOnce({ success: true, data: {} })
        .mockResolvedValueOnce({
          success: true,
          data: {
            entries: [userEntry("e2", "latest")],
            hasMore: true,
            nextOffset: 20,
          },
        })
        .mockRejectedValueOnce(new Error("timeout"))
        .mockResolvedValueOnce({
          success: true,
          data: {
            entries: [userEntry("e1", "older")],
            hasMore: false,
            nextOffset: 0,
          },
        });
      render();
      await establish();
      await act(async () => {
        expect(await result.current.loadOlderTimeline()).toBe(false);
      });
      expect(result.current.canLoadOlderTimeline).toBe(true);
      expect(result.current.loadingOlderTimeline).toBe(false);
      const page: { ids: false | string[] } = { ids: false };
      await act(async () => {
        page.ids = await result.current.loadOlderTimeline();
      });
      await flush();
      expect(page.ids).toEqual(expect.arrayContaining([expect.any(String)]));
      expect(result.current.timeline.items.map((item) => item.id)).toEqual(
        expect.arrayContaining(page.ids === false ? [] : page.ids),
      );
      expect(request.mock.calls[2][0].before).toBe(20);
      expect(request.mock.calls[3][0].before).toBe(20);
      expect(result.current.canLoadOlderTimeline).toBe(false);
      errorSpy.mockRestore();
    });

    test("rejects a non-advancing backward cursor", async () => {
      const errorSpy = jest
        .spyOn(console, "error")
        .mockImplementation(() => {});
      options.selectedSessionId = "s1";
      options.selectedRef.current = "s1";
      request
        .mockResolvedValueOnce({ success: true, data: {} })
        .mockResolvedValueOnce({
          success: true,
          data: {
            entries: [userEntry("e2", "latest")],
            hasMore: true,
            nextOffset: 20,
          },
        })
        .mockResolvedValueOnce({
          success: true,
          data: {
            entries: [userEntry("e1", "older")],
            hasMore: true,
            nextOffset: 20,
          },
        });
      render();
      await establish();
      await act(async () => {
        await result.current.loadOlderTimeline();
      });
      expect(errorSpy).toHaveBeenCalledWith(
        "[remote] older history page failed",
        expect.objectContaining({ before: 20 }),
      );
      expect(result.current.canLoadOlderTimeline).toBe(true);
      expect(result.current.loadingOlderTimeline).toBe(false);
      errorSpy.mockRestore();
    });
  });

  describe("engine deps", () => {
    test("requestGetState throws when the client is absent", async () => {
      options.selectedSessionId = "s1";
      const errorSpy = jest
        .spyOn(console, "error")
        .mockImplementation(() => {});
      options.clientRef.current = null;
      render();
      await act(async () => {
        result.current.reconcileSession("s1", "open");
      });
      await flush();
      expect(errorSpy).toHaveBeenCalledWith(
        "[remote] session timeline sync failed",
        expect.objectContaining({
          error: expect.objectContaining({ message: "not_connected" }),
        }),
      );
      errorSpy.mockRestore();
    });

    test("requestGetState fetches state and replays a run", async () => {
      options.selectedSessionId = "s1";
      request.mockImplementation(async (cmd: { type: string }) => {
        if (cmd.type === "get_state")
          return { success: true, data: { activeRun: { runId: "r1" } } };
        if (cmd.type === "get_session_entries")
          return {
            success: true,
            data: { entries: [userEntry("e1", "hi")], hasMore: false },
          };
        if (cmd.type === "get_events_since")
          return {
            success: true,
            data: {
              events: [
                { type: "agent_start", data: "{}", runId: "r1", idx: 0 },
                {
                  type: "text_chunk",
                  data: JSON.stringify({ text: "replayed" }),
                  runId: "r1",
                  idx: 1,
                },
                { type: "agent_end", data: "{}", runId: "r1", idx: 2 },
              ],
              hasMore: false,
            },
          };
        return { success: true, data: {} };
      });
      render();
      await establish();
      const texts = result.current.timeline.items
        .filter((i) => i.kind === "message")
        .map((i) => (i.kind === "message" ? i.text : ""));
      expect(texts).toContain("replayed");
    });

    test("history from a replaced client is rejected before replay", async () => {
      const errorSpy = jest
        .spyOn(console, "error")
        .mockImplementation(() => {});
      options.selectedSessionId = "s1";
      request.mockImplementation(async (cmd: { type: string }) => {
        if (cmd.type === "get_state")
          return { success: true, data: { activeRun: { runId: "r1" } } };
        if (cmd.type === "get_session_entries") {
          // Replacing the client invalidates this in-flight history page.
          options.clientRef.current = null;
          return {
            success: true,
            data: { entries: [userEntry("e1", "hi")], hasMore: false },
          };
        }
        return { success: true, data: {} };
      });
      render();
      await establish();
      expect(errorSpy).toHaveBeenCalledWith(
        "[remote] session timeline sync failed",
        expect.objectContaining({
          stage: "history",
          error: expect.objectContaining({ message: "stale_sync_lane" }),
        }),
      );
      errorSpy.mockRestore();
    });

    test("onRecovered clears a timed-out session error", async () => {
      jest.useFakeTimers();
      try {
        options.selectedSessionId = "s1";
        options.selectedRef.current = "s1";
        options.clientRef.current = null;
        render();
        await act(async () => {
          await jest.advanceTimersByTimeAsync(15_001);
        });
        expect(result.current.timelineError).toBe("timeout");
        options.clientRef.current = makeClient();
        request.mockResolvedValue({
          success: true,
          data: { entries: [userEntry("e1", "hi")], hasMore: false },
        });
        await act(async () => {
          result.current.reconcileSession("s1", "open");
          await jest.advanceTimersByTimeAsync(0);
        });
        await act(async () => {
          await Promise.resolve();
        });
        expect(result.current.timelineError).toBeNull();
      } finally {
        jest.useRealTimers();
      }
    });
  });

  describe("applySessionStreaming", () => {
    test.each([false, true])(
      "checks authoritative state before accepting a running snapshot (active=%s)",
      async (active) => {
        options.selectedSessionId = "s1";
        request.mockImplementation(async (command: { type: string }) => {
          if (command.type === "get_state")
            return { data: { activeRun: active ? { runId: "r" } : null } };
          if (command.type === "get_session_entries")
            return { data: { entries: [] } };
          return {
            data: {
              events: [{ type: "agent_start", runId: "r", idx: 0, data: "{}" }],
            },
          };
        });
        render();
        const engine = result.current.syncEngineRef.current!;
        engine.mutate("s1", () => ({
          ...emptyTimeline(),
          items: [
            {
              id: "m1",
              kind: "message" as const,
              role: "assistant" as const,
              text: "x",
            },
          ],
        }));
        await flush();
        act(() => result.current.applySessionStreaming("s1", true));
        // A stale catalog hint must never immediately resurrect the stop button.
        expect(result.current.timeline.streaming).toBe(false);
        await flush();
        expect(result.current.timeline.streaming).toBe(active);
        expect(result.current.syncEngineRef.current!.streamingFor("s1")).toBe(
          active,
        );
      },
    );

    test("does not reconcile when streaming state is unchanged", () => {
      render();
      const engine = result.current.syncEngineRef.current!;
      const reconcile = jest.spyOn(engine, "reconcile");
      result.current.applySessionStreaming("s1", false);
      expect(reconcile).not.toHaveBeenCalled();
    });

    test("reconciles on a snapshot flip to not-streaming", () => {
      render();
      const engine = result.current.syncEngineRef.current!;
      const reconcile = jest.spyOn(engine, "reconcile");
      result.current.streamingRef.current["s1"] = true;
      result.current.applySessionStreaming("s1", false);
      expect(reconcile).toHaveBeenCalledWith("s1", "snapshot-flip", undefined);
    });

    test("is a no-op without an engine", () => {
      render();
      result.current.syncEngineRef.current = null;
      expect(() =>
        result.current.applySessionStreaming("s1", false),
      ).not.toThrow();
    });
  });

  describe("resetTimeline", () => {
    test("clears the engine and all timeline state", async () => {
      options.selectedSessionId = "s1";
      request.mockResolvedValue({
        success: true,
        data: { entries: [userEntry("e1", "hi")], hasMore: false },
      });
      render();
      await establish();
      expect(result.current.timeline.items.length).toBeGreaterThan(0);
      act(() => result.current.resetTimeline());
      expect(result.current.timeline.items).toEqual([]);
    });
  });

  describe("ensureDraftTimeline", () => {
    test("seeds an empty draft timeline when absent", () => {
      render();
      act(() => result.current.ensureDraftTimeline());
      expect(result.current.timeline.items).toEqual([]);
    });
  });

  describe("timeline load timeout", () => {
    test("marks a session as timed out after the deadline", async () => {
      jest.useFakeTimers();
      try {
        options.selectedSessionId = "s1";
        options.selectedRef.current = "s1";
        render();
        await act(async () => {
          await jest.advanceTimersByTimeAsync(15_001);
        });
        expect(result.current.timelineError).toBe("timeout");
      } finally {
        jest.useRealTimers();
      }
    });

    test("skips the timeout when the selected session has already changed", async () => {
      jest.useFakeTimers();
      try {
        options.selectedSessionId = "s1";
        options.selectedRef.current = "s1";
        render();
        // Switch the selection away before the deadline fires.
        options.selectedRef.current = "s2";
        await act(async () => {
          await jest.advanceTimersByTimeAsync(15_001);
        });
        expect(result.current.timelineError).toBeNull();
      } finally {
        jest.useRealTimers();
      }
    });

    test("clears the timer when the session resolves before the deadline", async () => {
      jest.useFakeTimers();
      try {
        options.selectedSessionId = "s1";
        request.mockResolvedValue({
          success: true,
          data: { entries: [userEntry("e1", "hi")], hasMore: false },
        });
        render();
        await flush();
        expect(result.current.timelineError).toBeNull();
      } finally {
        jest.useRealTimers();
      }
    });
  });

  describe("retryTimeline", () => {
    test("is a no-op when no session is selected", async () => {
      render();
      await act(async () => {
        await result.current.retryTimeline();
      });
      expect(request).not.toHaveBeenCalled();
    });

    test("recovers the client and restarts the session sync", async () => {
      options.selectedRef.current = "s1";
      render();
      const engine = result.current.syncEngineRef.current!;
      const restart = jest.spyOn(engine, "restart");
      const recoverNow = (
        client().current as unknown as { recoverNow: jest.Mock }
      ).recoverNow;
      await act(async () => {
        await result.current.retryTimeline();
      });
      expect(recoverNow).toHaveBeenCalledWith("request-failure");
      expect(restart).toHaveBeenCalledWith("s1", "open");
    });

    test("swallows a recover failure and still restarts", async () => {
      options.selectedRef.current = "s1";
      render();
      const engine = result.current.syncEngineRef.current!;
      const restart = jest.spyOn(engine, "restart");
      const recoverNow = (
        client().current as unknown as { recoverNow: jest.Mock }
      ).recoverNow;
      recoverNow.mockRejectedValueOnce(new Error("offline"));
      await act(async () => {
        await result.current.retryTimeline();
      });
      expect(restart).toHaveBeenCalledWith("s1", "open");
    });

    test("diagnosticError handles a non-Error recovery failure", async () => {
      const errorSpy = jest
        .spyOn(console, "error")
        .mockImplementation(() => {});
      options.selectedRef.current = "s1";
      render();
      const recoverNow = (
        client().current as unknown as { recoverNow: jest.Mock }
      ).recoverNow;
      recoverNow.mockRejectedValueOnce("plain string failure");
      await act(async () => {
        await result.current.retryTimeline();
      });
      expect(errorSpy).toHaveBeenCalledWith(
        "[remote] timeline retry transport recovery failed",
        expect.objectContaining({ error: { message: "plain string failure" } }),
      );
      errorSpy.mockRestore();
    });

    test("clears an existing timeout error before retrying", async () => {
      jest.useFakeTimers();
      try {
        options.selectedSessionId = "s1";
        options.selectedRef.current = "s1";
        options.clientRef.current = null;
        render();
        await act(async () => {
          await jest.advanceTimersByTimeAsync(15_001);
        });
        expect(result.current.timelineError).toBe("timeout");
        options.clientRef.current = makeClient();
        await act(async () => {
          await result.current.retryTimeline();
        });
        expect(result.current.timelineError).toBeNull();
      } finally {
        jest.useRealTimers();
      }
    });
  });

  describe("manual compaction outcome", () => {
    test("session identity and multiple early results are preserved", async () => {
      render();
      for (const op of ["mine", "other"]) {
        act(() => result.current.handleEvent(evt("compaction_committed", JSON.stringify({ operation_id: op })), "s1"));
      }
      await expect(result.current.awaitCompactionOutcome("s1", "mine")).resolves.toEqual({ status: "committed" });
      const settled = jest.fn();
      const wait = result.current.awaitCompactionOutcome("s2", "other");
      void wait.then(settled);
      await flush();
      expect(settled).not.toHaveBeenCalled();
      act(() => result.current.handleEvent(evt("compaction_failed", JSON.stringify({ operation_id: "other", error: "own result" })), "s2"));
      await expect(wait).resolves.toEqual({ status: "failed", error: "own result" });
    });

    test.each(["abort", "reset", "unmount"])("%s settles a wait and releases all timers", async how => {
      jest.useFakeTimers();
      try {
        render();
        const baselineTimers = jest.getTimerCount();
        const controller = new AbortController();
        const wait = result.current.awaitCompactionOutcome("s1", "op", 30_000, controller.signal);
        expect(jest.getTimerCount()).toBe(baselineTimers + 2);
        if (how === "abort") act(() => controller.abort());
        if (how === "reset") act(() => result.current.resetTimeline());
        if (how === "unmount") { act(() => renderer!.unmount()); renderer = null; }
        await expect(wait).resolves.toEqual({ status: "cancelled" });
        expect(jest.getTimerCount()).toBe(how === "unmount" ? 0 : baselineTimers);
      } finally { jest.useRealTimers(); }
    });

    test("a read started before registration cannot settle the new operation", async () => {
      let read!: (value: unknown) => void;
      request.mockImplementation((cmd: { type: string }) => cmd.type === "get_state"
        ? new Promise(resolve => { read = resolve; }) : Promise.resolve({ data: { entries: [] } }));
      render();
      await establish();
      const done = jest.fn();
      const wait = result.current.awaitCompactionOutcome("s1", "op");
      void wait.then(done);
      await act(async () => read({ data: { isCompacting: false } }));
      await flush();
      expect(done).not.toHaveBeenCalled();
      act(() => result.current.handleEvent(evt("compaction_committed", '{"operation_id":"op"}'), "s1"));
      await expect(wait).resolves.toEqual({ status: "committed" });
    });

    test("a projected falling flag is not authoritative completion evidence", async () => {
      request.mockImplementation(async (cmd: { type: string }) => ({ data: cmd.type === "get_state" ? { isCompacting: true } : { entries: [] } }));
      render();
      await establish();
      const done = jest.fn();
      const wait = result.current.awaitCompactionOutcome("s1", "mine");
      void wait.then(done);
      act(() => result.current.syncEngineRef.current!.mutate("s1", state => ({ ...state, compacting: false })));
      await flush();
      expect(done).not.toHaveBeenCalled();
      act(() => result.current.handleEvent(evt("compaction_committed", '{"operation_id":"mine"}'), "s1"));
      await expect(wait).resolves.toEqual({ status: "committed" });
    });

    test("read-only probing recovers when both started and terminal frames are lost", async () => {
      jest.useFakeTimers();
      try {
        request.mockImplementation(async (cmd: { type: string }) => ({ data: cmd.type === "get_state" ? { isCompacting: false } : { entries: [] } }));
        render();
        await establish();
        const wait = result.current.awaitCompactionOutcome("s1", "lost");
        await act(async () => { await jest.advanceTimersByTimeAsync(5_001); });
        await expect(wait).resolves.toEqual({ status: "unobserved" });
        expect(request.mock.calls.every(([cmd]) => cmd.type.startsWith("get_"))).toBe(true);
      } finally { jest.useRealTimers(); }
    });
    const terminal = (type: string, data: Record<string, unknown>) => evt(type, JSON.stringify(data));

    test("a stale probe cannot strand a wait registered after its own settle", async () => {
      const reads: Array<(value: unknown) => void> = [];
      request.mockImplementation((cmd: { type: string }) => cmd.type === "get_state"
        ? new Promise(resolve => { reads.push(resolve as (value: unknown) => void); })
        : Promise.resolve({ data: { entries: [] } }));
      render();
      // A wait starts before the session opens, so the opening state read
      // snapshots it.
      const first = result.current.awaitCompactionOutcome("s1", "op", 600_000);
      await establish();
      expect(reads.length).toBe(1);
      const staleRead = reads[0]!;
      // The terminal arrives while that read is still pending, settling the wait
      // and removing it from the registry.
      act(() => result.current.handleEvent(terminal("compaction_committed", { operation_id: "op" }), "s1"));
      await expect(first).resolves.toEqual({ status: "committed" });
      // A second wait for the same operation is live under the same key.
      const second = result.current.awaitCompactionOutcome("s1", "op", 600_000);
      // The stale read finally answers idle. It must not touch the wait that is
      // live now: settling (or unregistering) the wait it snapshotted would
      // strand this one until its own timeout.
      await act(async () => { staleRead({ data: { isCompacting: false } }); await flush(); });
      act(() => result.current.handleEvent(terminal("compaction_committed", { operation_id: "op" }), "s1"));
      await expect(second).resolves.toEqual({ status: "committed" });
    });

    test("settles on the matching operation and ignores another operation's event", async () => {
      render();
      await establish();
      const settled = jest.fn();
      let outcome: Promise<unknown> | null = null;
      act(() => {
        outcome = result.current.awaitCompactionOutcome("s1", "cmp-2");
        void outcome.then(settled);
      });
      act(() => result.current.handleEvent(terminal("compaction_committed", { operation_id: "cmp-1" }), "s1"));
      await flush();
      expect(settled).not.toHaveBeenCalled();
      act(() => result.current.handleEvent(terminal("compaction_committed", { operation_id: "cmp-2" }), "s1"));
      await flush();
      await expect(outcome).resolves.toEqual({ status: "committed" });
    });

    test("keeps a terminal event that arrives before its initiator correlates it", async () => {
      render();
      await establish();
      act(() => result.current.handleEvent(terminal("compaction_failed", {
        operation_id: "cmp-early",
        error: "summary failed",
      }), "s1"));
      await flush();
      await expect(result.current.awaitCompactionOutcome("s1", "cmp-early"))
        .resolves.toEqual({ status: "failed", error: "summary failed" });
    });

    test("reports a missing terminal event as timeout, never as failure", async () => {
      jest.useFakeTimers();
      try {
        render();
        await establish();
        const outcome = result.current.awaitCompactionOutcome("s1", "cmp-slow", 5_000);
        await act(async () => {
          await jest.advanceTimersByTimeAsync(5_001);
        });
        await expect(outcome).resolves.toEqual({ status: "timeout" });
      } finally {
        jest.useRealTimers();
      }
    });

    test("ends the wait when the session stops compacting without a terminal frame", async () => {
      // The phone can miss the terminal frame (backgrounded, dropped stream).
      // The authoritative state ends the wait, so a lost frame cannot leave the
      // composer refusing new requests for the whole timeout.
      request.mockImplementation(async (command: { type: string }) => ({ data:
        command.type === "get_state" ? { isCompacting: true } : { entries: [] },
      }));
      render();
      await establish();
      act(() => result.current.handleEvent(
        terminal("compaction_started", { operation_id: "cmp-lost", phase: "standalone" }),
        "s1",
      ));
      await flush();
      const outcome = result.current.awaitCompactionOutcome("s1", "cmp-lost");
      request.mockImplementation(async (command: { type: string }) => ({ data:
        command.type === "get_state" ? { isCompacting: false } : { entries: [] },
      }));
      act(() => result.current.reconcileSession("s1", "reconnect"));
      await flush();
      await expect(outcome).resolves.toEqual({ status: "unobserved" });
    });

    test("carries the unchanged/reused flags and tolerates malformed payloads", async () => {
      render();
      await establish();
      act(() => result.current.handleEvent(terminal("compaction_unchanged", {
        operation_id: "cmp-reused",
        already_compacted: true,
        reused: true,
      }), "s1"));
      await flush();
      await expect(result.current.awaitCompactionOutcome("s1", "cmp-reused"))
        .resolves.toEqual({ status: "unchanged", alreadyCompacted: true, reused: true });
      // A malformed terminal frame must not settle a waiter with garbage.
      act(() => result.current.handleEvent(evt("compaction_failed", "not-json"), "s1"));
      await flush();
      act(() => result.current.handleEvent(terminal("compaction_failed", { error: "no id" }), "s1"));
      await flush();
      const outcome = result.current.awaitCompactionOutcome("s1", "cmp-none", 1);
      await act(async () => {
        await new Promise(resolve => setTimeout(resolve, 5));
      });
      await expect(outcome).resolves.toEqual({ status: "timeout" });
    });
  });

  describe("paging and refresh edges", () => {
    const terminal = (type: string, data: Record<string, unknown>) =>
      evt(type, JSON.stringify(data));

    test("a pull-to-refresh with no selected session is a no-op, and with one rebuilds that lane", async () => {
      render();
      await establish();
      const engine = result.current.syncEngineRef.current!;
      const restart = jest.spyOn(engine, "restart");
      // Nothing is selected: restarting would open a session that is not the
      // one on screen. The escape hatch must be inert rather than guess.
      options.selectedRef.current = "";
      act(() => result.current.reloadTimeline());
      expect(restart).not.toHaveBeenCalled();
      options.selectedRef.current = "s1";
      act(() => result.current.reloadTimeline());
      expect(restart).toHaveBeenCalledWith("s1", "open");
    });

    test("seeding a paging window reaches both the ref and the rendered state", () => {
      options.selectedSessionId = "s1";
      options.selectedRef.current = "s1";
      render();
      // The seam exists so paging tests need not drive several real round
      // trips. If it wrote only the ref, the screen would never see the
      // window and `canLoadOlderTimeline` would stay false.
      const window = { nextBefore: 12, endOffset: 3, hasMore: true, loading: false };
      act(() => result.current.seedHistoryPaging("s1", window));
      expect(result.current.historyPaging.s1).toEqual(window);
      expect(result.current.canLoadOlderTimeline).toBe(true);
      expect(result.current.loadingOlderTimeline).toBe(false);
    });

    test("the parking lot for early compaction terminals is bounded and keeps the newest operations", async () => {
      render();
      await establish();
      // Terminals that arrive with no initiator (a replay, or a screen that
      // opened mid-compaction) are parked for a later await. A long session
      // replays many operations, so the lot must evict rather than grow.
      for (let i = 0; i < 33; i += 1) {
        act(() => result.current.handleEvent(
          terminal("compaction_committed", { operation_id: `cmp-${i}`, phase: "standalone" }),
          "s1",
        ));
      }
      request.mockImplementation(async (command: { type: string }) => ({
        data: command.type === "get_state" ? { isCompacting: false } : { entries: [] },
      }));
      // The oldest is gone: only a real probe can answer for it, and with the
      // session idle that probe ends in a timeout.
      const evicted = result.current.awaitCompactionOutcome("s1", "cmp-0", 1);
      await act(async () => { await new Promise(resolve => setTimeout(resolve, 5)); });
      await expect(evicted).resolves.toEqual({ status: "timeout" });
      // The newest is still remembered, without a probe.
      await expect(result.current.awaitCompactionOutcome("s1", "cmp-32"))
        .resolves.toEqual({ status: "committed" });
    });

    test("a trimmed tail whose backfill page does not end flush fails the refresh instead of installing a hole", async () => {
      options.selectedSessionId = "s1";
      options.selectedRef.current = "s1";
      // The tail page is byte-trimmed and short of the exchange budget, so the
      // controller backfills. That second page claims a cursor that does not
      // join the requested range: the durable history moved underneath the
      // read, so nothing may be installed — not even the tail that did arrive.
      request.mockImplementation(async (command: { type: string; before?: number }) => {
        if (command.type !== "get_session_entries") return { data: {} };
        return command.before === Number.MAX_SAFE_INTEGER
          ? { data: { entries: [userEntry("u-tail", "tail question")], nextOffset: 4, hasMore: true, trimmed: true } }
          : { data: { entries: [userEntry("u-back", "backfill question")], nextOffset: 2, hasMore: false } };
      });
      render();
      await establish();
      await flush();
      const texts = result.current.timeline.items
        .filter((item) => item.kind === "message")
        .map((item) => (item.kind === "message" ? item.text : ""));
      expect(texts).not.toContain("tail question");
      expect(texts).not.toContain("backfill question");
    });

    test("a session switch while an older page is in flight restores the paging window and drops the stale page", async () => {
      options.selectedSessionId = "s1";
      options.selectedRef.current = "s1";
      render();
      await establish();
      await flush(80);
      act(() => result.current.seedHistoryPaging("s1", {
        nextBefore: 20, endOffset: 0, hasMore: true, loading: false,
      }));
      let release!: (value: unknown) => void;
      request.mockImplementation(() => new Promise((resolve) => { release = resolve; }));
      const pending = result.current.loadOlderTimeline();
      await flush(1);
      expect(result.current.loadingOlderTimeline).toBe(true);
      release({ data: { entries: [userEntry("u-old", "older question")], nextOffset: 17, hasMore: true } });
      // Let the page read finish, then leave the session before the committed
      // page can be installed.
      await Promise.resolve();
      options.selectedRef.current = "s2";
      await act(async () => { await pending; });
      options.selectedRef.current = "s1";
      expect(result.current.loadingOlderTimeline).toBe(false);
      expect(result.current.canLoadOlderTimeline).toBe(true);
      expect(result.current.timeline.items.some(
        (item) => item.kind === "message" && item.text === "older question",
      )).toBe(false);
    });

    /** A page the bridge returned short of the requested window. */
    const page = (
      entries: ReturnType<typeof userEntry>[],
      nextOffset: number,
      hasMore: boolean,
      trimmed = false,
    ) => ({ data: { entries, nextOffset, hasMore, ...(trimmed ? { trimmed } : {}) } });

    const rows = (prefix: string, count: number, start: number) =>
      Array.from({ length: count }, (_, i) => userEntry(`${prefix}-${start + i}`, `${prefix} ${start + i}`));

    const messageTexts = () => result.current.timeline.items
      .filter((item) => item.kind === "message")
      .map((item) => (item.kind === "message" ? item.text : ""));

    test("a refresh supersedes an older page still in flight so the stale page cannot land", async () => {
      options.selectedSessionId = "s1";
      options.selectedRef.current = "s1";
      request.mockImplementation((command: { type: string; before?: number }) =>
        command.type !== "get_session_entries"
          ? Promise.resolve({ data: {} })
          : command.before === 20
            // The older page the user asked for never lands on its own.
            ? new Promise(() => {})
            : Promise.resolve(page(rows("tail", 1, 8), 8, true)));
      render();
      await establish();
      act(() => result.current.seedHistoryPaging("s1", {
        nextBefore: 20, endOffset: 8, hasMore: true, loading: false,
      }));
      const pending = result.current.loadOlderTimeline();
      await flush();
      // A refresh rebuilds the window from the tail. The page still in flight
      // belongs to the cursor that was just replaced, so it is abandoned
      // rather than committed into the fresh window.
      act(() => result.current.reconcileSession("s1", "resend"));
      await flush();
      await act(async () => { await pending; });
      expect(messageTexts()).toContain("tail 8");
    });

    test("a gap page that is short only because of the byte trim is bridged from an untrimmed re-read", async () => {
      options.selectedSessionId = "s1";
      options.selectedRef.current = "s1";
      let tails = 0;
      request.mockImplementation(async (command: { type: string; before?: number; untrimmed?: boolean }) => {
        if (command.type !== "get_session_entries") return { data: {} };
        const before = command.before ?? 0;
        if (before === Number.MAX_SAFE_INTEGER) {
          tails += 1;
          // The cold open sees the same tail the refresh will.
          return page(rows("row", 1, 8), 8, true);
        }
        // The trimmed page sheds the older exchanges of the range it was asked
        // for and jumps its cursor past them, so it cannot end flush.
        if (before === 8 && command.untrimmed !== true) return page(rows("row", 1, 7), 4, true);
        if (before === 8) return page(rows("row", 3, 5), 5, true);
        return page(rows("row", 5, 0), 0, false);
      });
      render();
      await establish();
      act(() => result.current.seedHistoryPaging("s1", {
        nextBefore: 30, endOffset: 0, hasMore: true, loading: false,
      }));
      act(() => result.current.reconcileSession("s1", "resend"));
      await flush();
      // Nothing was lost between the paged window and the fresh tail, and the
      // shed range came back exactly once: the untrimmed re-read is the
      // authoritative version of the range, not an addition to it.
      expect(tails).toBeGreaterThan(1);
      expect(messageTexts()).toEqual(
        Array.from({ length: 9 }, (_, i) => `row ${i}`),
      );
    });

    test("a gap page that is short without hasMore is corruption and fails the refresh", async () => {
      options.selectedSessionId = "s1";
      options.selectedRef.current = "s1";
      let tails = 0;
      request.mockImplementation(async (command: { type: string; before?: number; untrimmed?: boolean }) => {
        if (command.type !== "get_session_entries") return { data: {} };
        const before = command.before ?? 0;
        if (before === Number.MAX_SAFE_INTEGER) {
          tails += 1;
          return tails === 1
            ? page(rows("open", 1, 0), 0, false)
            : page(rows("row", 1, 8), 8, true);
        }
        // No trim, no more history, yet the page does not reach the cursor:
        // the journal moved underneath the read, so nothing may be installed.
        return page(rows("row", 1, 7), 4, false);
      });
      render();
      await establish();
      act(() => result.current.seedHistoryPaging("s1", {
        nextBefore: 30, endOffset: 0, hasMore: true, loading: false,
      }));
      act(() => result.current.reconcileSession("s1", "resend"));
      await flush();
      // The failed refresh left the conversation it could not rebuild alone.
      expect(messageTexts()).toEqual(["open 0"]);
    });

    test("an untrimmed re-read that still does not end flush fails the refresh", async () => {
      options.selectedSessionId = "s1";
      options.selectedRef.current = "s1";
      let tails = 0;
      request.mockImplementation(async (command: { type: string; before?: number; untrimmed?: boolean }) => {
        if (command.type !== "get_session_entries") return { data: {} };
        const before = command.before ?? 0;
        if (before === Number.MAX_SAFE_INTEGER) {
          tails += 1;
          return tails === 1
            ? page(rows("open", 1, 0), 0, false)
            : page(rows("row", 1, 8), 8, true);
        }
        if (before === 8 && command.untrimmed !== true) return page(rows("row", 1, 7), 4, true);
        // Opting out of the trim was supposed to make the whole range
        // available. It did not, so the gap cannot be trusted even after the
        // retry — the untrimmed page is validated like any other.
        return page(rows("bad", 2, 6), 4, true);
      });
      render();
      await establish();
      act(() => result.current.seedHistoryPaging("s1", {
        nextBefore: 30, endOffset: 0, hasMore: true, loading: false,
      }));
      act(() => result.current.reconcileSession("s1", "resend"));
      await flush();
      expect(messageTexts()).toEqual(["open 0"]);
    });
  });

  /**
   * Guards and completion paths the happy flow never reaches: a retained paging
   * window with no durable prefix, a client that disappears mid-request, a lane
   * that goes stale as its page commits, a complete run that needs no replay,
   * and the compaction waiter's own timing edges.
   */
  describe("lane and waiter edges", () => {
    const compactionEvent = (type: string, operationId: string) =>
      evt(type, JSON.stringify({ operation_id: operationId }));
    const texts = () => result.current.timeline.items
      .filter((item) => item.kind === "message")
      .map((item) => (item.kind === "message" ? item.text : ""));

    test("a retained window with no durable prefix keeps only the fresh page", async () => {
      jest.useFakeTimers();
      try {
        options.selectedSessionId = "s1";
        options.selectedRef.current = "s1";
        request.mockResolvedValue({ data: {} });
        render();
        await establish();
        // Drain the open reconcile's own bookkeeping (its commit republishes the
        // paging window) before seeding the window this test drives.
        await act(async () => { await jest.advanceTimersByTimeAsync(0); });
        await act(async () => { await jest.advanceTimersByTimeAsync(0); });
        // A live-only item: no durable id and no durable run, so nothing of it can
        // be proven to belong to the retained window.
        act(() => result.current.syncEngineRef.current!.mutate("s1", live =>
          commitAcknowledgedUserMessage(live, { id: "local:1", runId: "r1", text: "typed" })));
        await act(async () => { await jest.advanceTimersByTimeAsync(0); });
        expect(result.current.timeline.items.map(item => item.id)).toContain("local:1");
        request.mockImplementation(async (cmd: { type: string }) => cmd.type === "get_session_entries"
          ? { data: { entries: [userEntry("h1", "durable")], nextOffset: 5, hasMore: false } }
          : { data: {} });
        await act(async () => {
          // The retained window ends exactly where the fresh page begins (an exact
          // join) and holds no durable item, so there is no prefix to keep.
          result.current.seedHistoryPaging("s1", {
            nextBefore: 20, endOffset: 5, hasMore: true, loading: false,
          });
          result.current.reconcileSession("s1", "reconnect");
          await jest.advanceTimersByTimeAsync(0);
        });
        await act(async () => { await jest.advanceTimersByTimeAsync(0); });
        expect(texts()).toContain("durable");
        // The fresh page alone was adopted, which hands paging over to its own
        // cursor. Keeping an empty prefix instead would keep claiming the old
        // window is open, and the reader would be offered a page that is gone.
        expect(result.current.canLoadOlderTimeline).toBe(false);
      } finally { jest.useRealTimers(); }
    });

    test("agent_end on the open session re-reads the list even without a catalogue event", () => {
      options.selectedSessionId = "s1";
      options.selectedRef.current = "s1";
      render();
      (options.refreshSessions as jest.Mock).mockClear();
      result.current.handleEvent(evt("agent_end", "{}"), "s1");
      // The completion is only visible in the catalogue, and nothing else will
      // ask for it: the session list would otherwise keep showing "running".
      expect(options.refreshSessions).toHaveBeenCalled();
    });

    test("a client that disappears before the history read yields an empty timeline", async () => {
      options.selectedSessionId = "s1";
      options.selectedRef.current = "s1";
      request.mockImplementation(async (cmd: { type: string }) => {
        // The connection is torn down between get_state and the history page.
        if (cmd.type === "get_state") options.clientRef.current = null;
        return { data: { entries: [userEntry("h1", "durable")], nextOffset: 0, hasMore: false } };
      });
      render();
      act(() => result.current.reconcileSession("s1", "open"));
      await flush();
      // No page may be requested without a client, and the timeline must stay
      // empty rather than keeping rows from a connection that is gone.
      expect(request.mock.calls.some(([cmd]) => cmd.type === "get_session_entries")).toBe(false);
      expect(result.current.timeline.items).toEqual([]);
    });

    test("a lane that goes stale as its page commits does not advance the cursor", async () => {
      jest.useFakeTimers();
      try {
        options.selectedSessionId = "s1";
        options.selectedRef.current = "s1";
        request.mockResolvedValue({ data: { entries: [], nextOffset: 0, hasMore: false } });
        render();
        await establish();
        // Drain the open reconcile's own bookkeeping (its commit republishes the
        // paging window) before seeding the window this test drives.
        await act(async () => { await jest.advanceTimersByTimeAsync(0); });
        await act(async () => { await jest.advanceTimersByTimeAsync(0); });
        request.mockResolvedValue({ data: {
          entries: [userEntry("u-old", "older")], nextOffset: 17, hasMore: true,
        } });
        // The commit that publishes the page is exactly where the reader leaves:
        // its subscriber runs in the same microtask as the resolve, before the
        // awaiting caller resumes, which is how a navigation interleaves with a
        // commit. Only the page's own commit counts.
        let flipped = false;
        const unsubscribe = result.current.syncEngineRef.current!.subscribe(commit => {
          if (commit.sessionId === "s1" &&
              commit.timeline.items.some(item => item.kind === "message" && item.text === "older")) {
            flipped = true;
            options.selectedRef.current = "s2";
          }
        });
        let applied: unknown;
        const requestsBefore = request.mock.calls.length;
        await act(async () => {
          // Seeded and read in one turn: the lane's pending work cannot slip
          // between the window and the read that uses it.
          result.current.seedHistoryPaging("s1", {
            nextBefore: 20, endOffset: 0, hasMore: true, loading: false,
          });
          applied = await result.current.loadOlderTimeline();
        });
        unsubscribe();
        expect(request.mock.calls.length).toBeGreaterThan(requestsBefore);
        expect(flipped).toBe(true);
        // The page was downloaded, and the commit that published it is where the
        // reader left: the cursor and the paging window must not move for a
        // conversation the user is no longer on.
        expect(applied).toBe(false);
      } finally { jest.useRealTimers(); }
    });

    test("a replay asked for without a client fails as not connected", async () => {
      const errorSpy = jest.spyOn(console, "error").mockImplementation(() => {});
      options.selectedSessionId = "s1";
      options.selectedRef.current = "s1";
      let arm = false;
      request.mockImplementation(async (cmd: { type: string }) => {
        // The desktop answers get_state, and the connection is gone the moment
        // that reply is processed — before the run's replay is asked for.
        if (cmd.type === "get_state") {
          if (arm) queueMicrotask(() => { options.clientRef.current = null; });
          return { success: true, data: { activeRun: { runId: "r1" } } };
        }
        if (cmd.type === "get_events_since") {
          return { success: true, data: {
            events: [
              { type: "agent_start", data: "{}", runId: "r1", idx: 0 },
              { type: "text_chunk", data: JSON.stringify({ text: "whole" }), runId: "r1", idx: 1 },
              { type: "agent_end", data: "{}", runId: "r1", idx: 2 },
            ],
            hasMore: false,
          } };
        }
        return { success: true, data: { entries: [userEntry("h1", "durable")], hasMore: false } };
      });
      render();
      await establish();
      await flush(80);
      // Ask again while the socket has just died. The run's prefix is already
      // complete, so no history read precedes the replay it needs — only the
      // replay path can notice the missing connection.
      const historyBefore = request.mock.calls.filter(([cmd]) => cmd.type === "get_session_entries").length;
      arm = true;
      act(() => result.current.reconcileSession("s1", "snapshot-flip", "r1"));
      await flush(60);
      expect(request.mock.calls.filter(([cmd]) => cmd.type === "get_session_entries").length)
        .toBe(historyBefore);
      expect(errorSpy).toHaveBeenCalledWith(
        "[remote] session timeline sync failed",
        expect.objectContaining({
          stage: "replay",
          error: expect.objectContaining({ message: "not_connected" }),
        }),
      );
      errorSpy.mockRestore();
    });

    test("a snapshot flip that names an already-complete run is not re-read", async () => {
      const run = "run-complete";
      options.selectedSessionId = "s1";
      options.selectedRef.current = "s1";
      request.mockImplementation(async (cmd: { type: string }) => ({
        success: true,
        data: cmd.type === "get_state" ? { activeRun: { runId: run } }
          : cmd.type === "get_events_since" ? {
            events: [
              { type: "agent_start", data: "{}", runId: run, idx: 0 },
              { type: "text_chunk", data: JSON.stringify({ text: "whole" }), runId: run, idx: 1 },
              { type: "agent_end", data: "{}", runId: run, idx: 2 },
            ],
            hasMore: false,
          }
          : { entries: [userEntry("h1", "ask")], hasMore: false },
      }));
      render();
      await establish();
      const engine = result.current.syncEngineRef.current!;
      expect(engine.runCompleteLocally("s1", run)).toBe(true);
      const reconcile = jest.spyOn(engine, "reconcile");
      // The catalogue still thinks the run is live for one snapshot.
      result.current.streamingRef.current["s1"] = true;
      act(() => result.current.applySessionStreaming("s1", false));
      // The terminal already arrived over a complete prefix: re-reading the run
      // would show a sync notice for text that is on screen.
      expect(reconcile).not.toHaveBeenCalled();
    });

    test("a compaction wait that is already aborted never registers", async () => {
      render();
      const controller = new AbortController();
      controller.abort();
      await expect(
        result.current.awaitCompactionOutcome("s1", "op", 60_000, controller.signal),
      ).resolves.toEqual({ status: "cancelled" });
      // No waiter and no probe: the caller had already given up.
      expect(request.mock.calls.filter(([cmd]) => cmd.type === "get_state")).toHaveLength(0);
    });

    test("a second compaction wait for the same operation shares the first promise", async () => {
      render();
      const first = result.current.awaitCompactionOutcome("s1", "op", 60_000);
      const second = result.current.awaitCompactionOutcome("s1", "op", 60_000);
      // One operation, one waiter: a second ask must not replace the first
      // promise, or the outcome the first caller is awaiting would be lost.
      expect(second).toBe(first);
      act(() => result.current.handleEvent(compactionEvent("compaction_committed", "op"), "s1"));
      await expect(first).resolves.toEqual({ status: "committed" });
    });

    test("an unresolved compaction wait keeps probing until its terminal arrives", async () => {
      jest.useFakeTimers();
      try {
        options.selectedSessionId = "s1";
        options.selectedRef.current = "s1";
        // The desktop says it is still compacting, so the probe proves nothing
        // and the wait has to keep watching.
        request.mockImplementation(async (cmd: { type: string }) => ({
          success: true,
          data: cmd.type === "get_state" ? { isCompacting: true } : { entries: [] },
        }));
        render();
        await act(async () => { await jest.advanceTimersByTimeAsync(0); });
        const wait = result.current.awaitCompactionOutcome("s1", "slow");
        const probes = () => request.mock.calls.filter(([cmd]) => cmd.type === "get_state").length;
        await act(async () => { await jest.advanceTimersByTimeAsync(5_000); });
        const afterFirstPoll = probes();
        expect(afterFirstPoll).toBeGreaterThan(0);
        await act(async () => { await jest.advanceTimersByTimeAsync(5_000); });
        // Each unresolved probe arms exactly one more: the wait keeps watching
        // while the desktop still reports the compaction running.
        expect(probes()).toBeGreaterThan(afterFirstPoll);
        act(() => result.current.handleEvent(compactionEvent("compaction_committed", "slow"), "s1"));
        await expect(wait).resolves.toEqual({ status: "committed" });
        // The poll that follows a settled wait must find it finished instead of
        // probing the desktop again.
        const settled = probes();
        await act(async () => { await jest.advanceTimersByTimeAsync(20_000); });
        expect(probes()).toBe(settled);
      } finally { jest.useRealTimers(); }
    });
  });
});
