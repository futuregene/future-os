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
  type TimelineState,
} from "./timeline";
import type { EntriesData, RemoteSessionState, StreamEvent } from "./types";

const TIMELINE_LOAD_TIMEOUT_MS = 15_000;
const HISTORY_PAGE_USER_EXCHANGES = 10;
const HISTORY_TAIL_CURSOR = Number.MAX_SAFE_INTEGER;

interface HistoryPagingState {
  nextBefore: number;
  hasMore: boolean;
  loading: boolean;
}

function latestTimelineWindow(timeline: TimelineState, userExchangeCount: number): TimelineState {
  let remaining = userExchangeCount;
  let start = 0;
  for (let index = timeline.items.length - 1; index >= 0; index -= 1) {
    const item = timeline.items[index];
    if (item?.kind !== "message" || item.role !== "user") continue;
    remaining -= 1;
    if (remaining === 0) {
      start = index;
      break;
    }
  }
  if (remaining > 0 || start === 0) return timeline;
  const items = timeline.items.slice(start);
  const visibleIds = new Set(items.map(item => item.id));
  return {
    ...timeline,
    items,
    durableItemIds: new Set([...(timeline.durableItemIds ?? [])].filter(id => visibleIds.has(id))),
  };
}

function prependHistoryPage(live: TimelineState, older: TimelineState): TimelineState {
  const liveIds = new Set(live.items.map(item => item.id));
  const olderItems = older.items.filter(item => !liveIds.has(item.id));
  return {
    ...live,
    items: [...olderItems, ...live.items],
    durableItemIds: new Set([...(live.durableItemIds ?? []), ...(older.durableItemIds ?? [])]),
  };
}

/** Replace the already-loaded tail with a fresh durable page while retaining
 * the older prefix the user explicitly paged in. Entry ids are stable across
 * history reads, so the first overlap is the exact splice point. */
function retainOlderHistoryPrefix(
  existing: TimelineState | null,
  latest: TimelineState,
): TimelineState {
  if (!existing || latest.items.length === 0) return latest;
  const latestIds = new Set(latest.items.map(item => item.id));
  const overlap = existing.items.findIndex(item => latestIds.has(item.id));
  if (overlap <= 0) return latest;
  const prefix = existing.items.slice(0, overlap);
  return {
    ...latest,
    items: [...prefix, ...latest.items],
    durableItemIds: new Set([
      ...(latest.durableItemIds ?? []),
      ...prefix.filter(item => existing.durableItemIds?.has(item.id)).map(item => item.id),
    ]),
  };
}

function diagnosticError(error: unknown): {
  name?: string;
  message: string;
  code?: unknown;
} {
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
  const historyPagingRef = useRef<Record<string, HistoryPagingState>>({});
  const historyEpochRef = useRef(0);
  const [historyPaging, setHistoryPaging] = useState<Record<string, HistoryPagingState>>({});
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
      if (sid !== selectedRef.current) {
        // Background conversations need catalog/unread/approval status, not
        // token projection or eager history/replay. Opening reloads durable
        // history and the active prefix, including any deferred approvals.
        if (["agent_end", "approval_request", "approval_decision"].includes(event.type))
          void refreshSessions();
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
    [reconcileSession, refreshModels, refreshSessions, selectedRef, setTitleOverrides],
  );

  const loadHistory = useCallback(
    async (sessionId: string): Promise<TimelineState> => {
      const client = clientRef.current;
      if (!client) return emptyTimeline();
      const epoch = historyEpochRef.current;
      const retained = historyPagingRef.current[sessionId];
      const response = await client.requestRetry<EntriesData>(
        {
          type: "get_session_entries",
          sessionId,
          before: HISTORY_TAIL_CURSOR,
          limit: HISTORY_PAGE_USER_EXCHANGES,
        },
        sessionId,
      );
      if (epoch !== historyEpochRef.current || clientRef.current !== client || selectedRef.current !== sessionId)
        throw new Error("stale_history_load");
      const entries = response.data.entries ?? [];
      const nextBefore = response.data.nextOffset ?? 0;
      const latest = timelineFromEntries(entries);
      const history = retained
        ? retainOlderHistoryPrefix(syncEngineRef.current?.timelineFor(sessionId) ?? null, latest)
        : latest;
      // Only retain the cursor when the old prefix actually joined this page.
      // Otherwise the latest page starts a new contiguous history window.
      const retainedOlderPages = history !== latest ? retained : null;
      const page: HistoryPagingState = {
        nextBefore: retainedOlderPages?.nextBefore ?? nextBefore,
        hasMore: retainedOlderPages?.hasMore ?? (response.data.hasMore === true && nextBefore > 0),
        loading: false,
      };
      historyPagingRef.current[sessionId] = page;
      setHistoryPaging(previous => ({ ...previous, [sessionId]: page }));
      return history;
    },
    [clientRef, selectedRef],
  );

  const loadOlderTimeline = useCallback(async () => {
    const sessionId = selectedRef.current;
    const current = historyPagingRef.current[sessionId];
    const client = clientRef.current;
    if (!sessionId || !client || !current?.hasMore || current.loading) return false;

    const loading = { ...current, loading: true };
    historyPagingRef.current[sessionId] = loading;
    setHistoryPaging(previous => ({ ...previous, [sessionId]: loading }));
    try {
      const response = await client.requestRetry<EntriesData>(
        {
          type: "get_session_entries",
          sessionId,
          before: current.nextBefore,
          limit: HISTORY_PAGE_USER_EXCHANGES,
        },
        sessionId,
      );
      // Reopening/reconciling may have replaced this cursor while the request
      // was in flight. Never install an old page into that new paging window.
      if (historyPagingRef.current[sessionId] !== loading) return false;
      const entries = response.data.entries ?? [];
      const nextBefore = response.data.nextOffset ?? 0;
      if (response.data.hasMore && (nextBefore <= 0 || nextBefore >= current.nextBefore)) {
        throw new Error("history_backward_cursor_not_advancing");
      }
      const next: HistoryPagingState = {
        nextBefore,
        hasMore: response.data.hasMore === true && nextBefore > 0,
        loading: false,
      };
      historyPagingRef.current[sessionId] = next;
      setHistoryPaging(previous => ({ ...previous, [sessionId]: next }));
      const older = timelineFromEntries(entries);
      syncEngineRef.current?.mutate(sessionId, live => prependHistoryPage(live, older));
      // The sync lane may still be busy. The view waits until these exact
      // message ids reach its committed data before declaring paging complete.
      return older.items.map(item => item.id);
    } catch (error) {
      if (historyPagingRef.current[sessionId] !== loading) return false;
      const failed = { ...current, loading: false };
      historyPagingRef.current[sessionId] = failed;
      setHistoryPaging(previous => ({ ...previous, [sessionId]: failed }));
      console.error("[remote] older history page failed", {
        sessionId,
        before: current.nextBefore,
        error: diagnosticError(error),
      });
      return false;
    }
  }, [clientRef, selectedRef]);

  const prepareTimelineOpen = useCallback((sessionId: string) => {
    historyEpochRef.current += 1;
    const cached = timelinesRef.current[sessionId];
    if (!cached) return;
    const windowed = latestTimelineWindow(cached, HISTORY_PAGE_USER_EXCHANGES);
    if (windowed === cached) return;

    // A reopened conversation may have many explicitly paged rows in memory.
    // Keep cache warmth, but restore the same bounded first-render window as a
    // cold open; the authoritative tail reconcile below will restore its cursor.
    const nextTimelines = { ...timelinesRef.current, [sessionId]: windowed };
    timelinesRef.current = nextTimelines;
    setTimelines(nextTimelines);
    syncEngineRef.current?.mutate(sessionId, () => windowed);

    const nextPaging = { ...historyPagingRef.current };
    delete nextPaging[sessionId];
    historyPagingRef.current = nextPaging;
    setHistoryPaging(nextPaging);
  }, []);

  useEffect(() => {
    const engine = new SyncEngine({
      isSessionVisible: sessionId => sessionId === selectedRef.current,
      requestGetState: async sessionId => {
        const client = clientRef.current;
        if (!client) throw new Error("not_connected");
        return (
          await client.requestRetry<RemoteSessionState>({ type: "get_state", sessionId }, sessionId)
        ).data;
      },
      requestHistory: loadHistory,
      fetchReplay: async (sessionId, runId, sinceIdx, isCurrent) => {
        const client = clientRef.current;
        if (!client) throw new Error("not_connected");
        const merged = await fetchEventsSince(client, sessionId, runId, sinceIdx,
          () => clientRef.current === client && isCurrent());
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
  }, [clientRef, loadHistory, selectedRef]);

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
    historyEpochRef.current += 1;
    timelinesRef.current = {};
    syncEngineRef.current?.clear();
    setTimelines({});
    setTimelineErrors({});
    cursorsRef.current = {};
    streamingRef.current = {};
    historyPagingRef.current = {};
    setHistoryPaging({});
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
  const selectedHistoryPaging = selectedSessionId ? historyPaging[selectedSessionId] : undefined;

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
      setTimelineErrors(previous => ({
        ...previous,
        [sessionId]: "timeout",
      }));
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
    canLoadOlderTimeline: selectedHistoryPaging?.hasMore ?? false,
    loadingOlderTimeline: selectedHistoryPaging?.loading ?? false,
    loadOlderTimeline,
    prepareTimelineOpen,
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
