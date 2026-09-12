import { createElement } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import type { RemoteClient } from "../client";
import { applyStreamEvent, emptyTimeline } from "../timeline";
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
function assistantEntry(id: string, text: string, runId?: string): HistoryEntry {
  return {
    id,
    role: "assistant",
    kind: "assistant",
    createdAtMs: 0,
    blocks: [{ kind: "text", text }],
    runId,
  };
}
function evt(type: string, data: string, runId?: string, idx?: number): StreamEvent {
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
    act(() => {
      result.current.reconcileSession(sessionId, "open");
    });
    await flush();
  }

  test("slow active-run replay does not trigger the 15-second history timeout", async () => {
    jest.useFakeTimers();
    options.selectedSessionId = "s1";
    options.selectedRef.current = "s1";
    request.mockImplementation(async (command: { type: string }) => {
      if (command.type === "get_state") return { data: { activeRun: { runId: "r" } } };
      if (command.type === "get_session_entries") return { data: { entries: [userEntry("u", "readable history")] } };
      await new Promise(resolve => setTimeout(resolve, 20_000));
      return { data: { events: [] } };
    });
    try {
      render();
      await establish();
      expect(result.current.timelinePending).toBe(false);
      expect(result.current.timeline.items).toHaveLength(1);
      await act(async () => { await jest.advanceTimersByTimeAsync(15_001); });
      expect(result.current.timelinePending).toBe(false);
      expect(result.current.timelineError).toBeNull();
      expect(result.current.timeline.items[0]).toMatchObject({ id: "m_u", text: "readable history" });
      await act(async () => { await jest.advanceTimersByTimeAsync(5_000); });
      expect(result.current.timelineError).toBeNull();
    } finally {
      act(() => renderer!.unmount());
      renderer = null;
      jest.useRealTimers();
    }
  });

  test.each(["restart", "resend"])(
    "%s refreshes an idle session and keeps disjoint history reachable",
    async mode => {
      options.selectedSessionId = "s1";
      options.selectedRef.current = "s1";
      const exchanges = (start: number, end: number) =>
        Array.from({ length: end - start + 1 }, (_, i) => start + i).flatMap(n => [
          userEntry(`u${n}`, `user ${n}`),
          assistantEntry(`a${n}`, `answer ${n}`),
        ]);
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
        });
      render();
      await establish();
      await act(async () => {
        await result.current.loadOlderTimeline();
      });
      await flush();
      act(() => {
        result.current.syncEngineRef.current!.mutate("s1", live =>
          applyStreamEvent(live, evt("user_message", JSON.stringify({ text: "pending" }))),
        );
      });
      await flush();
      act(() => {
        if (mode === "restart") result.current.syncEngineRef.current!.restartAll("reconnect");
        else result.current.reconcileSession("s1", "resend");
      });
      await flush();
      const users = result.current.timeline.items
        .filter(i => i.kind === "message" && i.role === "user")
        .map(i => (i.kind === "message" ? i.text : ""));
      expect(users).toEqual([...Array.from({ length: 10 }, (_, i) => `user ${31 + i}`), "pending"]);
      expect(result.current.canLoadOlderTimeline).toBe(true);
      request.mockResolvedValueOnce({
        success: true,
        data: { entries: exchanges(21, 30), hasMore: true, nextOffset: 40 },
      });
      await act(async () => {
        await result.current.loadOlderTimeline();
      });
      await flush();
      expect(request.mock.calls.at(-1)?.[0]).toEqual(
        expect.objectContaining({ before: 60, limit: 10 }),
      );
      expect(
        result.current.timeline.items
          .filter(i => i.kind === "message" && i.role === "user")
          .map(i => (i.kind === "message" ? i.text : "")),
      ).toEqual([...Array.from({ length: 20 }, (_, i) => `user ${21 + i}`), "pending"]);
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
    test("ignores events with an empty session id", () => {
      render();
      result.current.handleEvent(evt("agent_start", "{}"), "");
      expect(request).not.toHaveBeenCalled();
    });

    test("provider_config_changed triggers a model refresh", () => {
      render();
      result.current.handleEvent(evt("provider_config_changed", "{}"), "s1");
      expect(options.refreshModels).toHaveBeenCalled();
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
      const updater = (options.setTitleOverrides as jest.Mock).mock.calls[0][0] as (
        prev: Record<string, string>,
      ) => Record<string, string>;
      expect(updater({})).toEqual({ s1: "Renamed" });
      expect(options.refreshSessions).toHaveBeenCalled();
    });

    test("session_name_changed ignores a blank name", () => {
      render();
      result.current.handleEvent(evt("session_name_changed", JSON.stringify({ name: "  " })), "s1");
      expect(options.setTitleOverrides).not.toHaveBeenCalled();
      expect(options.refreshSessions).not.toHaveBeenCalled();
    });

    test("session_name_changed swallows malformed JSON", () => {
      render();
      result.current.handleEvent(evt("session_name_changed", "not json"), "s1");
      expect(options.setTitleOverrides).not.toHaveBeenCalled();
    });

    test("user_message hydrates attachments for the session", () => {
      render();
      const hydrate = jest.fn(async () => {});
      result.current.hydrateAttachmentsRef.current = hydrate;
      result.current.handleEvent(evt("user_message", JSON.stringify({ text: "hi" })), "s1");
      expect(hydrate).toHaveBeenCalledWith("s1");
    });

    test("approval_decision mutates a matching approval item", async () => {
      options.selectedSessionId = "s1";
      render();
      const engine = result.current.syncEngineRef.current!;
      engine.mutate("s1", tl => ({
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
        evt("approval_decision", JSON.stringify({ approval_request_id: "a1", status: "approved" })),
        "s1",
      );
      await flush();
      const approval = result.current.timeline.items.find(i => i.kind === "approval");
      expect(approval).toMatchObject({ decision: "approved" });
    });

    test("approval_decision leaves an unmatched approval item alone", async () => {
      options.selectedSessionId = "s1";
      render();
      const engine = result.current.syncEngineRef.current!;
      engine.mutate("s1", tl => ({
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
      const approval = result.current.timeline.items.find(i => i.kind === "approval");
      expect(approval?.decision).toBeUndefined();
    });

    test("approval_decision ignores an invalid status", () => {
      render();
      const engine = result.current.syncEngineRef.current!;
      const mutate = jest.spyOn(engine, "mutate");
      result.current.handleEvent(
        evt("approval_decision", JSON.stringify({ approval_request_id: "a1", status: "pending" })),
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
            entries: [userEntry("e3", "latest"), assistantEntry("e4", "answer")],
            hasMore: true,
            nextOffset: 20,
          },
        });
      render();
      await establish();
      const texts = result.current.timeline.items
        .filter(i => i.kind === "message")
        .map(i => (i.kind === "message" ? i.text : ""));
      expect(texts).toEqual(["latest", "answer"]);
      expect(request).toHaveBeenCalledTimes(2);
      expect(request.mock.calls[1]?.[0]).toEqual(
        expect.objectContaining({
          type: "get_session_entries",
          before: Number.MAX_SAFE_INTEGER,
          limit: 10,
        }),
      );
      expect(result.current.canLoadOlderTimeline).toBe(true);
    });

    test("loads one older page and prepends it without refetching the tail", async () => {
      options.selectedSessionId = "s1";
      options.selectedRef.current = "s1";
      request
        .mockResolvedValueOnce({ success: true, data: {} })
        .mockResolvedValueOnce({
          success: true,
          data: {
            entries: [userEntry("e3", "latest"), assistantEntry("e4", "answer")],
            hasMore: true,
            nextOffset: 20,
          },
        })
        .mockResolvedValueOnce({
          success: true,
          data: {
            entries: [userEntry("e1", "older"), assistantEntry("e2", "older answer")],
            hasMore: false,
            nextOffset: 0,
          },
        })
        .mockResolvedValueOnce({ success: true, data: {} })
        .mockResolvedValueOnce({
          success: true,
          data: {
            entries: [userEntry("e3", "latest"), assistantEntry("e4", "reconciled answer")],
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
        .filter(i => i.kind === "message")
        .map(i => (i.kind === "message" ? i.text : ""));
      expect(texts).toEqual(["older", "older answer", "latest", "answer"]);
      expect(request.mock.calls[2]?.[0]).toEqual(
        expect.objectContaining({
          type: "get_session_entries",
          before: 20,
          limit: 10,
        }),
      );
      expect(result.current.canLoadOlderTimeline).toBe(false);

      act(() => result.current.reconcileSession("s1", "resend"));
      await flush();
      const reconciledTexts = result.current.timeline.items
        .filter(i => i.kind === "message")
        .map(i => (i.kind === "message" ? i.text : ""));
      expect(reconciledTexts).toEqual(["older", "older answer", "latest", "reconciled answer"]);
      expect(request.mock.calls[4]?.[0]).toEqual(
        expect.objectContaining({ before: Number.MAX_SAFE_INTEGER, limit: 10 }),
      );
    });

    test("reopening a warm timeline renders only the latest ten exchanges", async () => {
      options.selectedSessionId = "s1";
      options.selectedRef.current = "s1";
      const exchanges = (start: number, end: number) =>
        Array.from({ length: end - start + 1 }, (_, i) => start + i).flatMap(n => [
          userEntry(`u${n}`, `user ${n}`),
          assistantEntry(`a${n}`, `answer ${n}`),
        ]);
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

      expect(
        result.current.timeline.items
          .filter(item => item.kind === "message" && item.role === "user")
          .map(item => (item.kind === "message" ? item.text : "")),
      ).toEqual(Array.from({ length: 10 }, (_, index) => `user ${index + 11}`));
      expect(result.current.canLoadOlderTimeline).toBe(false);
    });

    test("timeout retains the cursor and retry returns the committed page identities", async () => {
      const errorSpy = jest.spyOn(console, "error").mockImplementation(() => {});
      options.selectedSessionId = "s1";
      options.selectedRef.current = "s1";
      request
        .mockResolvedValueOnce({ success: true, data: {} })
        .mockResolvedValueOnce({
          success: true,
          data: { entries: [userEntry("e2", "latest")], hasMore: true, nextOffset: 20 },
        })
        .mockRejectedValueOnce(new Error("timeout"))
        .mockResolvedValueOnce({
          success: true,
          data: { entries: [userEntry("e1", "older")], hasMore: false, nextOffset: 0 },
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
      expect(result.current.timeline.items.map(item => item.id)).toEqual(
        expect.arrayContaining(page.ids === false ? [] : page.ids),
      );
      expect(request.mock.calls[2][0].before).toBe(20);
      expect(request.mock.calls[3][0].before).toBe(20);
      expect(result.current.canLoadOlderTimeline).toBe(false);
      errorSpy.mockRestore();
    });

    test("rejects a non-advancing backward cursor", async () => {
      const errorSpy = jest.spyOn(console, "error").mockImplementation(() => {});
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
      const errorSpy = jest.spyOn(console, "error").mockImplementation(() => {});
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
        .filter(i => i.kind === "message")
        .map(i => (i.kind === "message" ? i.text : ""));
      expect(texts).toContain("replayed");
    });

    test("fetchReplay throws when the client disappears before replay", async () => {
      const errorSpy = jest.spyOn(console, "error").mockImplementation(() => {});
      options.selectedSessionId = "s1";
      request.mockImplementation(async (cmd: { type: string }) => {
        if (cmd.type === "get_state")
          return { success: true, data: { activeRun: { runId: "r1" } } };
        if (cmd.type === "get_session_entries") {
          // Drop the client after history so fetchReplay sees null.
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
          error: expect.objectContaining({ message: "not_connected" }),
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
    test("flips streaming on a live timeline", async () => {
      options.selectedSessionId = "s1";
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
      result.current.applySessionStreaming("s1", true);
      await flush();
      expect(result.current.timeline.streaming).toBe(true);
    });

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
      expect(() => result.current.applySessionStreaming("s1", false)).not.toThrow();
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
      const recoverNow = (client().current as unknown as { recoverNow: jest.Mock }).recoverNow;
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
      const recoverNow = (client().current as unknown as { recoverNow: jest.Mock }).recoverNow;
      recoverNow.mockRejectedValueOnce(new Error("offline"));
      await act(async () => {
        await result.current.retryTimeline();
      });
      expect(restart).toHaveBeenCalledWith("s1", "open");
    });

    test("diagnosticError handles a non-Error recovery failure", async () => {
      const errorSpy = jest.spyOn(console, "error").mockImplementation(() => {});
      options.selectedRef.current = "s1";
      render();
      const recoverNow = (client().current as unknown as { recoverNow: jest.Mock }).recoverNow;
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
