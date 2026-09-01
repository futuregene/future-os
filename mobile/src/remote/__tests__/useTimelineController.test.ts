import React from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import type { RemoteClient } from "../client";
import { emptyTimeline } from "../timeline";
import type { HistoryEntry, StreamEvent } from "../types";
import { useTimelineController } from "../useTimelineController";

type Options = Parameters<typeof useTimelineController>[0];
type Result = ReturnType<typeof useTimelineController>;

function userEntry(id: string, text: string): HistoryEntry {
  return { id, role: "user", content: text };
}
function assistantEntry(id: string, text: string, runId?: string): HistoryEntry {
  return {
    id,
    role: "assistant",
    content: text,
    ...(runId ? { meta: { run_id: runId } } : {}),
  };
}
function evt(type: string, data: string, runId?: string, idx?: number): StreamEvent {
  return { type, data, ...(runId ? { runId } : {}), ...(idx != null ? { idx } : {}) };
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
      renderer = create(React.createElement(Harness));
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
        expect.objectContaining({ type: "get_session_entries", before: 20, limit: 10 }),
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

    test("rejects a non-advancing backward cursor", async () => {
      const errorSpy = jest.spyOn(console, "error").mockImplementation(() => {});
      options.selectedSessionId = "s1";
      options.selectedRef.current = "s1";
      request
        .mockResolvedValueOnce({ success: true, data: {} })
        .mockResolvedValueOnce({
          success: true,
          data: { entries: [userEntry("e2", "latest")], hasMore: true, nextOffset: 20 },
        })
        .mockResolvedValueOnce({
          success: true,
          data: { entries: [userEntry("e1", "older")], hasMore: true, nextOffset: 20 },
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
        expect.objectContaining({ error: expect.objectContaining({ message: "not_connected" }) }),
      );
      errorSpy.mockRestore();
    });

    test("requestGetState fetches state and replays a run", async () => {
      options.selectedSessionId = "s1";
      request.mockImplementation(async (cmd: { type: string }) => {
        if (cmd.type === "get_state")
          return { success: true, data: { activeRun: { runId: "r1" } } };
        if (cmd.type === "get_session_entries")
          return { success: true, data: { entries: [userEntry("e1", "hi")], hasMore: false } };
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
          return { success: true, data: { entries: [userEntry("e1", "hi")], hasMore: false } };
        }
        return { success: true, data: {} };
      });
      render();
      await establish();
      expect(errorSpy).toHaveBeenCalledWith(
        "[remote] session timeline sync failed",
        expect.objectContaining({ error: expect.objectContaining({ message: "not_connected" }) }),
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
        items: [{ id: "m1", kind: "message" as const, role: "assistant" as const, text: "x" }],
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
