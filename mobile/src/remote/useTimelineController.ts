import type { Dispatch, MutableRefObject, SetStateAction } from "react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { AppState } from "react-native";
import type { RemoteClient } from "./client";
import { fetchEventsSince } from "./replay";
import { requestReadPage } from "./readPages";
import type { RunCursor } from "./runCursor";
import {
  SyncEngine,
  type ReconcileReason,
  type TimelineSyncStatus,
} from "./syncEngine";
import {
  emptyTimeline,
  markApprovalDecision,
  mergeHistoryAttachments,
  timelineFromEntries,
  type TimelineState,
} from "./timeline";
import type { EntriesData, RemoteSessionState, StreamEvent } from "./types";

const TIMELINE_LOAD_TIMEOUT_MS = 15_000;
// One deadline covers transport retries AND waiting behind a replay. Expiry
// cancels the transaction, so late responses/queued mutations cannot skip pages.
const HISTORY_PAGE_TIMEOUT_MS = 30_000;
// An exchange can contain hundreds of tool steps. Bound both the first paint
// and each older-history pull to the same small window.
const HISTORY_PAGE_USER_EXCHANGES = 3;
const HISTORY_TAIL_CURSOR = Number.MAX_SAFE_INTEGER;

async function readHistoryPage(
  client: RemoteClient,
  sessionId: string,
  before: number,
  isCurrent: () => boolean,
): Promise<EntriesData> {
  const response = await requestReadPage<EntriesData>(client, {
    type: "get_session_entries", sessionId, before, limit: HISTORY_PAGE_USER_EXCHANGES,
  }, sessionId, isCurrent);
  return response.data;
}

type HistoryPagingState = NonNullable<TimelineState["historyWindow"]> & { loading: boolean };

function latestTimelineWindow(
  timeline: TimelineState,
  userExchangeCount: number,
): TimelineState {
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
  const visibleIds = new Set(items.map((item) => item.id));
  return {
    ...timeline,
    items,
    durableItemIds: new Set(
      [...(timeline.durableItemIds ?? [])].filter((id) => visibleIds.has(id)),
    ),
  };
}

function prependHistoryPage(
  live: TimelineState,
  older: TimelineState,
): TimelineState {
  const liveIds = new Set(live.items.map((item) => item.id));
  const olderItems = older.items.filter((item) => !liveIds.has(item.id));
  return {
    ...live,
    items: [...olderItems, ...live.items],
    durableItemIds: new Set([
      ...(live.durableItemIds ?? []),
      ...(older.durableItemIds ?? []),
    ]),
  };
}

/** A page is complete when the authoritative sync lane publishes its merge.
 * React rendering and native layout are deliberately outside this contract.
 */
function commitHistoryPage(
  engine: SyncEngine,
  sessionId: string,
  older: TimelineState,
  isCurrent: () => boolean,
  signal: AbortSignal,
): Promise<void> {
  return new Promise((resolve, reject) => {
    let applied = false;
    const cleanup = () => {
      unsubscribe();
      signal.removeEventListener("abort", abort);
    };
    const abort = () => {
      cleanup();
      reject(new Error("history_page_cancelled"));
    };
    const unsubscribe = engine.subscribe((commit) => {
      if (commit.sessionId !== sessionId || !applied) return;
      cleanup();
      resolve();
    });
    signal.addEventListener("abort", abort);
    if (!isCurrent()) {
      abort();
      return;
    }
    engine.mutate(sessionId, (live) => {
      if (!isCurrent()) {
        abort();
        return live;
      }
      applied = true;
      return prependHistoryPage(live, older);
    });
  });
}

/** Replace the loaded tail without evicting a contiguous older prefix. A long
 * exchange can make the bridge return only the next exchange: in that case
 * the durable entry cursor, not a shared UI row, proves the pages are adjacent. */
function retainOlderHistoryPrefix(
  existing: TimelineState | null,
  latest: TimelineState,
  adjacent: boolean,
): TimelineState {
  if (!existing || latest.items.length === 0) return latest;
  const latestIds = new Set(latest.items.map((item) => item.id));
  const latestRuns = new Set(latest.items.flatMap(item =>
    item.kind === "message" && item.runId ? [`${item.role}:${item.runId}`] : [],
  ));
  // A just-acknowledged prompt has a local id until history arrives. Bind it
  // by role + run, never by text (repeated prompts are distinct exchanges).
  const overlap = existing.items.findIndex(item => latestIds.has(item.id) ||
    (adjacent && item.kind === "message" && !!item.runId && latestRuns.has(`${item.role}:${item.runId}`)),
  );
  if (overlap === 0 || (overlap < 0 && !adjacent)) return latest;
  const durableRuns = new Set(existing.items.flatMap(item =>
    existing.durableItemIds?.has(item.id) && item.runId ? [item.runId] : [],
  ));
  const prefix = overlap < 0
    ? existing.items.filter(item => existing.durableItemIds?.has(item.id) ||
      (!!item.runId && durableRuns.has(item.runId)))
    : existing.items.slice(0, overlap);
  if (prefix.length === 0) return latest;
  return {
    ...latest,
    items: [...prefix, ...latest.items],
    durableItemIds: new Set([
      ...(latest.durableItemIds ?? []),
      ...prefix
        .filter((item) => existing.durableItemIds?.has(item.id))
        .map((item) => item.id),
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
      ...(error.cause &&
      typeof error.cause === "object" &&
      "code" in error.cause
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
  onSessionState?(sessionId: string, state: RemoteSessionState): void;
}

export function useTimelineController({
  clientRef,
  selectedRef,
  selectedSessionId,
  draft,
  refreshModels,
  refreshSessions,
  setTitleOverrides,
  onSessionState,
}: TimelineControllerOptions) {
  const onSessionStateRef = useRef(onSessionState);
  useEffect(() => { onSessionStateRef.current = onSessionState; }, [onSessionState]);
  const settingsRevisionRef = useRef(0);
  const settingsReadRef = useRef(0);
  const [timelines, setTimelines] = useState<Record<string, TimelineState>>({});
  const [timelineErrors, setTimelineErrors] = useState<
    Record<string, "timeout">
  >({});
  const [syncStatuses, setSyncStatuses] = useState<
    Record<string, TimelineSyncStatus>
  >({});
  const timelinesRef = useRef<Record<string, TimelineState>>({});
  const syncEngineRef = useRef<SyncEngine | null>(null);
  const cursorsRef = useRef<Record<string, RunCursor>>({});
  const streamingRef = useRef<Record<string, boolean>>({});
  const historyPagingRef = useRef<Record<string, HistoryPagingState>>({});
  const committedHistoryRef = useRef<Record<string, TimelineState["historyWindow"]>>({});
  const historyEpochRef = useRef(0);
  const olderRequestRef = useRef<{
    sessionId: string;
    controller: AbortController;
  } | null>(null);
  const [historyPaging, setHistoryPaging] = useState<
    Record<string, HistoryPagingState>
  >({});
  const hydrateAttachmentsRef = useRef<(sessionId: string) => Promise<void>>(
    async () => undefined,
  );

  useEffect(() => {
    if (olderRequestRef.current?.sessionId !== selectedSessionId)
      olderRequestRef.current?.controller.abort();
  }, [selectedSessionId]);
  useEffect(
    () => () => {
      olderRequestRef.current?.controller.abort();
    },
    [],
  );

  useEffect(() => {
    timelinesRef.current = timelines;
  }, [timelines]);

  const reconcileSession = useCallback(
    (
      sessionId: string | undefined,
      reason: ReconcileReason,
      runId?: string,
    ) => {
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
      if (event.type === "provider_config_changed" || event.type === "model_visibility_changed") {
        void refreshModels();
        return;
      }
      if (event.type === "model_changed" || event.type === "thinking_level_changed") {
        if (sid === selectedRef.current) settingsRevisionRef.current += 1;
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
            setTitleOverrides((previous) => ({ ...previous, [sid]: name }));
            void refreshSessions();
          }
        } catch {
          // Ignore malformed rename payloads.
        }
        return;
      }
      if (sid !== selectedRef.current || AppState.currentState === "background") {
        // Background conversations need catalog/unread/approval status, not
        // token projection or eager history/replay. Opening reloads durable
        // history and the active prefix, including any deferred approvals.
        if (
          ["agent_end", "approval_request", "approval_decision"].includes(
            event.type,
          )
        )
          void refreshSessions();
        return;
      }
      if (event.type === "user_message")
        void hydrateAttachmentsRef.current(sid);
      if (event.type === "approval_decision") {
        try {
          const data = JSON.parse(event.data) as Record<string, unknown>;
          const approvalId = data.approval_request_id;
          const status = data.status;
          if (
            approvalId &&
            (status === "approved" ||
              status === "rejected" ||
              status === "cancelled")
          ) {
            const decision = status as "approved" | "rejected" | "cancelled";
            const approvalRequestId = approvalId as string;
            syncEngineRef.current?.mutate(sid, (timeline) => {
              const has = timeline.items.some(
                (item) =>
                  item.kind === "approval" &&
                  item.payload.approval_request_id === approvalRequestId,
              );
              return has
                ? markApprovalDecision(timeline, approvalRequestId, decision)
                : timeline;
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
    [
      reconcileSession,
      refreshModels,
      refreshSessions,
      selectedRef,
      setTitleOverrides,
    ],
  );

  const loadHistory = useCallback(
    async (sessionId: string, laneIsCurrent: () => boolean = () => true): Promise<TimelineState> => {
      const client = clientRef.current;
      if (!client) return emptyTimeline();
      const epoch = historyEpochRef.current;
      const isCurrent = () => laneIsCurrent() && epoch === historyEpochRef.current &&
        clientRef.current === client && selectedRef.current === sessionId;
      const response = await readHistoryPage(client, sessionId, HISTORY_TAIL_CURSOR, isCurrent);
      let entries = response.entries ?? [];
      let nextBefore = response.nextOffset ?? 0;
      let hasMore = response.hasMore === true && nextBefore > 0;
      const endOffset = nextBefore + entries.length;
      // A newer durable window supersedes any page still waiting to commit.
      if (olderRequestRef.current?.sessionId === sessionId)
        olderRequestRef.current.controller.abort();
      const retained = historyPagingRef.current[sessionId];
      // Live events can append hundreds of entries without moving the last
      // history cursor. A capped tail then starts beyond that cursor, not
      // exactly beside it. Fill only this intervening range before replacing
      // the visible timeline; a failure leaves the old messages/cursor intact.
      while (retained && nextBefore > retained.endOffset) {
        const older = await readHistoryPage(client, sessionId, nextBefore, isCurrent);
        const olderEntries = older.entries ?? [];
        const start = older.nextOffset ?? 0;
        if (!Number.isSafeInteger(start) || start < 0 || start >= nextBefore ||
          start + olderEntries.length !== nextBefore) {
          throw new Error("history_gap_cursor_invalid");
        }
        entries = [...olderEntries, ...entries];
        nextBefore = start;
        hasMore = older.hasMore === true && start > 0;
      }
      if (!isCurrent()) throw new Error("stale_history_load");
      const latest = timelineFromEntries(entries);
      const history = retained
        ? retainOlderHistoryPrefix(
            syncEngineRef.current?.timelineFor(sessionId) ?? null,
            latest,
            retained.endOffset === nextBefore,
          )
        : latest;
      // Only retain the cursor when the old prefix actually joined this page.
      // Otherwise the latest page starts a new contiguous history window.
      const retainedOlderPages = history !== latest ? retained : null;
      return {
        ...history,
        historyWindow: {
          nextBefore: retainedOlderPages?.nextBefore ?? nextBefore,
          endOffset,
          hasMore: retainedOlderPages?.hasMore ?? hasMore,
        },
      };
    },
    [clientRef, selectedRef],
  );

  const loadOlderTimeline = useCallback(async () => {
    const sessionId = selectedRef.current;
    const current = historyPagingRef.current[sessionId];
    const client = clientRef.current;
    const engine = syncEngineRef.current;
    if (
      !sessionId ||
      !client ||
      !engine ||
      !current?.hasMore ||
      current.loading
    )
      return false;

    const controller = new AbortController();
    olderRequestRef.current?.controller.abort();
    olderRequestRef.current = { sessionId, controller };
    const loading = { ...current, loading: true };
    historyPagingRef.current[sessionId] = loading;
    setHistoryPaging((previous) => ({ ...previous, [sessionId]: loading }));
    const isCurrent = () =>
      !controller.signal.aborted &&
      historyPagingRef.current[sessionId] === loading &&
      clientRef.current === client &&
      selectedRef.current === sessionId &&
      syncEngineRef.current === engine;
    const timeout = setTimeout(
      () => controller.abort(),
      HISTORY_PAGE_TIMEOUT_MS,
    );
    let onAbort = () => {};
    const cancelled = new Promise<never>((_resolve, reject) => {
      onAbort = () => reject(new Error("history_page_cancelled"));
      controller.signal.addEventListener("abort", onAbort);
    });
    try {
      const response = await Promise.race([
        requestReadPage<EntriesData>(
          client,
          {
            type: "get_session_entries",
            sessionId,
            before: current.nextBefore,
            limit: HISTORY_PAGE_USER_EXCHANGES,
          },
          sessionId,
          isCurrent,
        ),
        cancelled,
      ]);
      // Reopening/reconciling may have replaced this cursor while the request
      // was in flight. Never install an old page into that new paging window.
      if (!isCurrent()) return false;
      const entries = response.data.entries ?? [];
      const nextBefore = response.data.nextOffset ?? 0;
      if (
        response.data.hasMore &&
        (nextBefore <= 0 || nextBefore >= current.nextBefore)
      ) {
        throw new Error("history_backward_cursor_not_advancing");
      }
      const older = timelineFromEntries(entries);
      await Promise.race([
        commitHistoryPage(
          engine,
          sessionId,
          older,
          isCurrent,
          controller.signal,
        ),
        cancelled,
      ]);
      if (!isCurrent()) return false;
      const next: HistoryPagingState = {
        nextBefore,
        endOffset: current.endOffset,
        hasMore: response.data.hasMore === true && nextBefore > 0,
        loading: false,
      };
      historyPagingRef.current[sessionId] = next;
      setHistoryPaging((previous) => ({ ...previous, [sessionId]: next }));
      // Advance the cursor only with a committed page, never with a merely
      // downloaded page which a restart or stalled replay could discard.
      return older.items.map((item) => item.id);
    } catch (error) {
      if (historyPagingRef.current[sessionId] !== loading) return false;
      const failed = { ...current, loading: false };
      historyPagingRef.current[sessionId] = failed;
      setHistoryPaging((previous) => ({ ...previous, [sessionId]: failed }));
      console.error("[remote] older history page failed", {
        sessionId,
        before: current.nextBefore,
        error: diagnosticError(error),
      });
      return false;
    } finally {
      clearTimeout(timeout);
      controller.signal.removeEventListener("abort", onAbort);
      if (olderRequestRef.current?.controller === controller)
        olderRequestRef.current = null;
      // Also fences a queued mutation if this operation failed before commit.
      controller.abort();
      if (historyPagingRef.current[sessionId] === loading) {
        historyPagingRef.current[sessionId] = current;
        setHistoryPaging((previous) => ({ ...previous, [sessionId]: current }));
      }
    }
  }, [clientRef, selectedRef]);

  const pruneTimelines = useCallback((selected: string) => {
    const removed = syncEngineRef.current?.pruneCache(selected) ?? [];
    if (removed.length === 0) return;
    const next = { ...timelinesRef.current };
    const paging = { ...historyPagingRef.current };
    for (const id of removed) {
      delete next[id];
      delete paging[id];
      delete committedHistoryRef.current[id];
      delete cursorsRef.current[id];
      delete streamingRef.current[id];
    }
    timelinesRef.current = next;
    historyPagingRef.current = paging;
    // A deferred prune can run after an engine commit queued a React update
    // but before the ref-mirroring effect. Remove only evicted keys from the
    // latest state; replacing it with the ref snapshot would erase that commit.
    setTimelines((previous) => {
      const retained = { ...previous };
      for (const id of removed) delete retained[id];
      return retained;
    });
    setHistoryPaging(paging);
    setSyncStatuses((previous) => {
      const next = { ...previous };
      for (const id of removed) delete next[id];
      return next;
    });
    setTimelineErrors((previous) => {
      const errors = { ...previous };
      for (const id of removed) delete errors[id];
      return errors;
    });
  }, []);

  useEffect(() => {
    // Cache sizing serializes timelines and can be expensive after a long run.
    // Do not do it in the navigation commit's effects: let the native screen
    // update first. Cancel stale cleanup when the user immediately reopens.
    const timer = setTimeout(() => {
      // Selection changes synchronously, before React cleans up this effect.
      if (selectedRef.current === selectedSessionId) pruneTimelines(selectedSessionId);
    }, 0);
    return () => clearTimeout(timer);
  }, [pruneTimelines, selectedRef, selectedSessionId]);

  const prepareTimelineOpen = useCallback(
    (sessionId: string) => {
      olderRequestRef.current?.controller.abort();
      historyEpochRef.current += 1;
      pruneTimelines(sessionId);
      // Explicit navigation starts a bounded latest window. Only an in-place
      // refresh bridges a gap back to already displayed history.
      const nextPaging = { ...historyPagingRef.current };
      delete nextPaging[sessionId];
      historyPagingRef.current = nextPaging;
      setHistoryPaging(nextPaging);
      const cached = timelinesRef.current[sessionId];
      if (!cached) return;
      const windowed = latestTimelineWindow(
        cached,
        HISTORY_PAGE_USER_EXCHANGES,
      );
      if (windowed === cached) return;

      // A reopened conversation may have many explicitly paged rows in memory.
      // Keep cache warmth, but restore the same bounded first-render window as a
      // cold open; the authoritative tail reconcile below will restore its cursor.
      const nextTimelines = { ...timelinesRef.current, [sessionId]: windowed };
      timelinesRef.current = nextTimelines;
      setTimelines(nextTimelines);
      syncEngineRef.current?.mutate(sessionId, () => windowed);
    },
    [pruneTimelines],
  );

  useEffect(() => {
    const engine = new SyncEngine({
      // The connection has a background grace period for pickers/quick app
      // switches, but there is no reason to project invisible text during it.
      // Foreground recovery already reconciles the missed durable suffix.
      isSessionVisible: (sessionId) => sessionId === selectedRef.current && AppState.currentState !== "background",
      requestGetState: async (sessionId) => {
        const client = clientRef.current;
        if (!client) throw new Error("not_connected");
        const revision = settingsRevisionRef.current;
        const read = ++settingsReadRef.current;
        const epoch = historyEpochRef.current;
        const state = (
          await client.requestRetry<RemoteSessionState>(
            { type: "get_state", sessionId },
            sessionId,
          )
        ).data;
        if (clientRef.current === client && selectedRef.current === sessionId
          && settingsRevisionRef.current === revision && settingsReadRef.current === read
          && historyEpochRef.current === epoch) {
          onSessionStateRef.current?.(sessionId, state);
        }
        return state;
      },
      requestHistory: loadHistory,
      fetchReplay: async (sessionId, runId, sinceIdx, isCurrent) => {
        const client = clientRef.current;
        if (!client) throw new Error("not_connected");
        const merged = await fetchEventsSince(
          client,
          sessionId,
          runId,
          sinceIdx,
          () => clientRef.current === client && isCurrent(),
        );
        return { ...merged, events: merged.events ?? [] };
      },
      onSyncStatus: (sessionId, status) => {
        setSyncStatuses((previous) =>
          previous[sessionId] === status
            ? previous
            : { ...previous, [sessionId]: status },
        );
      },
      onTiming: (timing) => {
        // Keep successful fast live-gap checks quiet in release builds. Slow
        // opens/retries still leave enough phase data for device diagnosis,
        // without recording prompts, reply text, or transport credentials.
        if (__DEV__ || timing.elapsedMs >= 1000) {
          // eslint-disable-next-line no-console -- Timings are diagnostics, not LogBox warnings.
          console.info("[remote] session timeline sync timing", timing);
        }
      },
      onFailure: (failure) => {
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
      onRecovered: (sessionId) => {
        setTimelineErrors((previous) => {
          if (!previous[sessionId]) return previous;
          const next = { ...previous };
          delete next[sessionId];
          return next;
        });
      },
    });
    const unsubscribe = engine.subscribe((commit) => {
      const window = commit.timeline.historyWindow;
      if (window && window !== committedHistoryRef.current[commit.sessionId]) {
        committedHistoryRef.current[commit.sessionId] = window;
        const page: HistoryPagingState = { ...window, loading: false };
        historyPagingRef.current[commit.sessionId] = page;
        setHistoryPaging(previous => ({ ...previous, [commit.sessionId]: page }));
      }
      setTimelines((previous) => {
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
    hydrateAttachmentsRef.current = async (sessionId) => {
      const engine = syncEngineRef.current;
      const client = clientRef.current;
      if (!engine || !client || !engine.timelineFor(sessionId)) return;
      const epoch = historyEpochRef.current;
      const isCurrent = () => clientRef.current === client && syncEngineRef.current === engine &&
        historyEpochRef.current === epoch && selectedRef.current === sessionId;
      try {
        // Attachment hydration does not install history rows. It must not
        // advance the durable paging cursor or cancel an older-page request.
        const page = await readHistoryPage(client, sessionId, HISTORY_TAIL_CURSOR, isCurrent);
        const durable = timelineFromEntries(page.entries ?? []);
        engine.mutate(sessionId, (live) =>
          isCurrent() ? mergeHistoryAttachments(live, durable) : live,
        );
      } catch {
        // Durable history can briefly lag the live event; later reconcile retries.
      }
    };
  }, [clientRef, selectedRef]);

  const applySessionStreaming = useCallback(
    (sessionId: string, streaming: boolean) => {
      const engine = syncEngineRef.current;
      if (!engine) return;
      const before = streamingRef.current[sessionId] ?? false;
      if (before === streaming) return;
      // Catalog snapshots are hints, not a second timeline writer. A delayed
      // running snapshot must not resurrect the stop button after agent_end.
      const run = !streaming
        ? engine.timelineFor(sessionId)?.currentRunId
        : undefined;
      engine.reconcile(sessionId, "snapshot-flip", run ?? undefined);
    },
    [],
  );

  const resetTimeline = useCallback(() => {
    olderRequestRef.current?.controller.abort();
    historyEpochRef.current += 1;
    timelinesRef.current = {};
    syncEngineRef.current?.clear();
    setTimelines({});
    setTimelineErrors({});
    setSyncStatuses({});
    cursorsRef.current = {};
    streamingRef.current = {};
    historyPagingRef.current = {};
    committedHistoryRef.current = {};
    setHistoryPaging({});
  }, []);

  const ensureDraftTimeline = useCallback(() => {
    setTimelines((previous) =>
      previous[""] ? previous : { ...previous, "": emptyTimeline() },
    );
  }, []);

  const timeline = useMemo(
    () => timelines[selectedSessionId || ""] ?? emptyTimeline(),
    [selectedSessionId, timelines],
  );
  const timelinePending = useMemo(
    () =>
      selectedSessionId !== "" &&
      !draft &&
      timelines[selectedSessionId] === undefined,
    [selectedSessionId, draft, timelines],
  );
  const timelineError = selectedSessionId
    ? (timelineErrors[selectedSessionId] ?? null)
    : null;
  const timelineSyncStatus: TimelineSyncStatus =
    selectedSessionId && !draft
      ? (syncStatuses[selectedSessionId] ?? "idle")
      : "idle";
  const selectedHistoryPaging = selectedSessionId
    ? historyPaging[selectedSessionId]
    : undefined;

  useEffect(() => {
    if (!timelinePending || !selectedSessionId || timelineError) return;
    const sessionId = selectedSessionId;
    const timer = setTimeout(() => {
      if (
        selectedRef.current !== sessionId ||
        timelinesRef.current[sessionId] !== undefined
      )
        return;
      console.error("[remote] session timeline load timed out", {
        sessionId,
        timeoutMs: TIMELINE_LOAD_TIMEOUT_MS,
      });
      setTimelineErrors((previous) => ({
        ...previous,
        [sessionId]: "timeout",
      }));
    }, TIMELINE_LOAD_TIMEOUT_MS);
    return () => clearTimeout(timer);
  }, [selectedRef, selectedSessionId, timelineError, timelinePending]);

  const retryTimeline = useCallback(async () => {
    const sessionId = selectedRef.current;
    if (!sessionId) return;
    setTimelineErrors((previous) => {
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
    if (selectedRef.current === sessionId)
      syncEngineRef.current?.restart(sessionId, "open");
  }, [clientRef, selectedRef]);

  return {
    timeline,
    timelinePending,
    timelineError,
    timelineSyncStatus,
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
