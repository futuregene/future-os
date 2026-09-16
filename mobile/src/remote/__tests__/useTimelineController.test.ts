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
});
