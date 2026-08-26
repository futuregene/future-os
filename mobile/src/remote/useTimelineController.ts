import type { Dispatch, MutableRefObject, SetStateAction } from "react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { RemoteClient } from "./client";
import { fetchEventsSince } from "./replay";
import type { RunCursor } from "./runCursor";
import { SyncEngine, type ReconcileReason } from "./syncEngine";
import {
  emptyTimeline,
  markApprovalDecision,
  mergeHistoryAttachments,
  timelineFromEntries,
  timelineFromHistory,
  type TimelineState,
} from "./timeline";
import type {
  EntriesData,
  HistoryData,
  HistoryEntry,
  HistoryMessage,
  RemoteSessionState,
  StreamEvent,
} from "./types";

const TIMELINE_LOAD_TIMEOUT_MS = 15_000;

function diagnosticError(error: unknown): { name?: string; message: string; code?: unknown } {
  if (error instanceof Error) {
    return {
      name: error.name,
      message: error.message,
      ...(error.cause && typeof error.cause === "object" && "code" in error.cause
        ? { code: (error.cause as { code?: unknown }).code }
        : {}),
    };
  }
  return { message: String(error) };
}

interface TimelineControllerOptions {
  clientRef: MutableRefObject<RemoteClient | null>;
  selectedRef: MutableRefObject<string>;
  selectedSessionId: string;
  draft: boolean;
  refreshModels(): Promise<void>;
  refreshSessions(): Promise<void>;
  setTitleOverrides: Dispatch<SetStateAction<Record<string, string>>>;
}

export function useTimelineController({
  clientRef,
  selectedRef,
  selectedSessionId,
  draft,
  refreshModels,
  refreshSessions,
  setTitleOverrides,
}: TimelineControllerOptions) {
  const [timelines, setTimelines] = useState<Record<string, TimelineState>>({});
  const [timelineErrors, setTimelineErrors] = useState<Record<string, "timeout">>({});
  const timelinesRef = useRef<Record<string, TimelineState>>({});
  const syncEngineRef = useRef<SyncEngine | null>(null);
  const cursorsRef = useRef<Record<string, RunCursor>>({});
  const streamingRef = useRef<Record<string, boolean>>({});
  const hydrateAttachmentsRef = useRef<(sessionId: string) => Promise<void>>(async () => undefined);

  useEffect(() => {
    timelinesRef.current = timelines;
  }, [timelines]);

  const reconcileSession = useCallback(
    (sessionId: string | undefined, reason: ReconcileReason, runId?: string) => {
      const engine = syncEngineRef.current;
      if (!engine) return;
      if (sessionId) engine.reconcile(sessionId, reason, runId);
      else engine.reconcileAll(reason);
    },
    [],
  );

  const handleEvent = useCallback(
    (event: StreamEvent, sessionId: string) => {
      const sid = sessionId || "";
      if (!sid) return;
      if (event.type === "provider_config_changed") {
        void refreshModels();
        return;
      }
      if (event.type === "run_snapshot") {
        reconcileSession(sid, "resend", event.runId ?? undefined);
        return;
      }
      if (event.type === "session_name_changed") {
        try {
          const data = JSON.parse(event.data) as Record<string, unknown>;
          const name = typeof data.name === "string" ? data.name.trim() : "";
          if (name) {
            setTitleOverrides(previous => ({ ...previous, [sid]: name }));
            void refreshSessions();
          }
        } catch {
          // Ignore malformed rename payloads.
        }
        return;
      }
      if (event.type === "user_message") void hydrateAttachmentsRef.current(sid);
      if (event.type === "approval_decision") {
        try {
          const data = JSON.parse(event.data) as Record<string, unknown>;
          const approvalId = data.approval_request_id;
          const status = data.status;
          if (
            approvalId &&
            (status === "approved" || status === "rejected" || status === "cancelled")
          ) {
            const decision = status as "approved" | "rejected" | "cancelled";
            const approvalRequestId = approvalId as string;
            syncEngineRef.current?.mutate(sid, timeline => {
              const has = timeline.items.some(
                item =>
                  item.kind === "approval" &&
                  item.payload.approval_request_id === approvalRequestId,
              );
              return has ? markApprovalDecision(timeline, approvalRequestId, decision) : timeline;
            });
          }
        } catch {
          // Ignore malformed decision payloads.
        }
        return;
      }
      syncEngineRef.current?.event(sid, event);
      if (event.type === "agent_end") void refreshSessions();
    },
    [reconcileSession, refreshModels, refreshSessions, setTitleOverrides],
  );

  const loadHistory = useCallback(
    async (sessionId: string): Promise<TimelineState> => {
      const client = clientRef.current;
      if (!client) return emptyTimeline();
      try {
        const entries: HistoryEntry[] = [];
        let offset = 0;
        for (;;) {
          const response = await client.requestRetry<EntriesData>(
            { type: "get_session_entries", sessionId, offset },
            sessionId,
          );
          entries.push(...(response.data.entries ?? []));
          if (!response.data.hasMore) break;
          const next = response.data.nextOffset;
          if (typeof next !== "number" || next <= offset) break;
          offset = next;
        }
        return timelineFromEntries(entries);
      } catch {
        // Older desktops fall back to message-shaped history.
      }
      const history: HistoryMessage[] = [];
      let offset = 0;
      for (;;) {
        const response = await client.requestRetry<HistoryData>(
          { type: "get_messages", sessionId, offset },
          sessionId,
        );
        history.push(...(response.data.messages ?? []));
        if (!response.data.hasMore) break;
        const next = response.data.nextOffset;
        if (typeof next !== "number" || next <= offset) break;
        offset = next;
      }
      return timelineFromHistory(history);
    },
    [clientRef],
  );

  useEffect(() => {
    const engine = new SyncEngine({
      requestGetState: async sessionId => {
        const client = clientRef.current;
        if (!client) throw new Error("not_connected");
        return (
          await client.requestRetry<RemoteSessionState>({ type: "get_state", sessionId }, sessionId)
        ).data;
      },
      requestHistory: loadHistory,
      fetchReplay: async (sessionId, runId, sinceIdx) => {
        const client = clientRef.current;
        if (!client) throw new Error("not_connected");
        const merged = await fetchEventsSince(client, sessionId, runId, sinceIdx);
        return { ...merged, events: merged.events ?? [] };
      },
      onFailure: failure => {
        console.error("[remote] session timeline sync failed", {
          sessionId: failure.sessionId,
          runId: failure.runId ?? null,
          reason: failure.reason,
          stage: failure.stage,
          attempt: failure.attempt,
          retryInMs: failure.retryInMs,
          error: diagnosticError(failure.error),
        });
      },
      onRecovered: sessionId => {
        setTimelineErrors(previous => {
          if (!previous[sessionId]) return previous;
          const next = { ...previous };
          delete next[sessionId];
          return next;
        });
      },
    });
    const unsubscribe = engine.subscribe(commit => {
      setTimelines(previous => {
        const existing = previous[commit.sessionId];
        return existing === commit.timeline
          ? previous
          : { ...previous, [commit.sessionId]: commit.timeline };
      });
      cursorsRef.current[commit.sessionId] = commit.cursor;
      streamingRef.current[commit.sessionId] = commit.timeline.streaming;
    });
    syncEngineRef.current = engine;
    return () => {
      unsubscribe();
      syncEngineRef.current = null;
      engine.clear();
    };
  }, [clientRef, loadHistory]);

  useEffect(() => {
    hydrateAttachmentsRef.current = async sessionId => {
      const engine = syncEngineRef.current;
      if (!engine || !engine.timelineFor(sessionId)) return;
      try {
        const durable = await loadHistory(sessionId);
        engine.mutate(sessionId, live => mergeHistoryAttachments(live, durable));
      } catch {
        // Durable history can briefly lag the live event; later reconcile retries.
      }
    };
  }, [loadHistory]);

  const applySessionStreaming = useCallback((sessionId: string, streaming: boolean) => {
    setTimelines(previous => {
      const existing = previous[sessionId];
      return existing && existing.streaming !== streaming
        ? { ...previous, [sessionId]: { ...existing, streaming } }
        : previous;
    });
    const engine = syncEngineRef.current;
    if (!engine) return;
    const before = streamingRef.current[sessionId] ?? false;
    if (before === streaming) return;
    streamingRef.current[sessionId] = streaming;
    if (!streaming) {
      const run = engine.timelineFor(sessionId)?.currentRunId;
      engine.reconcile(sessionId, "snapshot-flip", run ?? undefined);
    }
  }, []);

  const resetTimeline = useCallback(() => {
    syncEngineRef.current?.clear();
    setTimelines({});
    setTimelineErrors({});
    cursorsRef.current = {};
    streamingRef.current = {};
  }, []);

  const ensureDraftTimeline = useCallback(() => {
    setTimelines(previous => (previous[""] ? previous : { ...previous, "": emptyTimeline() }));
  }, []);

  const timeline = useMemo(
    () => timelines[selectedSessionId || ""] ?? emptyTimeline(),
    [selectedSessionId, timelines],
  );
  const timelinePending = useMemo(
    () => selectedSessionId !== "" && !draft && timelines[selectedSessionId] === undefined,
    [selectedSessionId, draft, timelines],
  );
  const timelineError = selectedSessionId ? (timelineErrors[selectedSessionId] ?? null) : null;

  useEffect(() => {
    if (!timelinePending || !selectedSessionId || timelineError) return;
    const sessionId = selectedSessionId;
    const timer = setTimeout(() => {
      if (selectedRef.current !== sessionId || timelinesRef.current[sessionId] !== undefined)
        return;
      console.error("[remote] session timeline load timed out", {
        sessionId,
        timeoutMs: TIMELINE_LOAD_TIMEOUT_MS,
      });
      setTimelineErrors(previous => ({ ...previous, [sessionId]: "timeout" }));
    }, TIMELINE_LOAD_TIMEOUT_MS);
    return () => clearTimeout(timer);
  }, [selectedRef, selectedSessionId, timelineError, timelinePending]);

  const retryTimeline = useCallback(async () => {
    const sessionId = selectedRef.current;
    if (!sessionId) return;
    setTimelineErrors(previous => {
      if (!previous[sessionId]) return previous;
      const next = { ...previous };
      delete next[sessionId];
      return next;
    });
    console.warn("[remote] retrying session timeline sync", { sessionId });
    const client = clientRef.current;
    if (client) {
      try {
        await client.recoverNow("request-failure");
      } catch (error) {
        console.error("[remote] timeline retry transport recovery failed", {
          sessionId,
          error: diagnosticError(error),
        });
      }
    }
    if (selectedRef.current === sessionId) syncEngineRef.current?.restart(sessionId, "open");
  }, [clientRef, selectedRef]);

  return {
    timeline,
    timelinePending,
    timelineError,
    syncEngineRef,
    streamingRef,
    hydrateAttachmentsRef,
    reconcileSession,
    handleEvent,
    applySessionStreaming,
    resetTimeline,
    ensureDraftTimeline,
    retryTimeline,
  };
}
