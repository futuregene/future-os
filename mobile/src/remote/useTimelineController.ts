import { useOlderHistory } from "./timeline/useOlderHistory";
import { compactionTerminalOf, useCompactionOutcomes } from "./timeline/useCompactionOutcomes";
import { HISTORY_PAGE_USER_EXCHANGES, HISTORY_TAIL_CURSOR, readHistoryPage, latestTimelineWindow, retainOlderHistoryPrefix, diagnosticError } from "./timeline/historyPaging";
import type { HistoryPagingState } from "./timeline/historyPaging";
import type { Dispatch, MutableRefObject, SetStateAction } from "react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { AppState } from "react-native";
import type { RemoteClient } from "./client";
import { fetchEventsSince } from "./replay";

import type { RunCursor } from "./runCursor";
import { SyncEngine, type ReconcileReason, type TimelineSyncStatus } from "./syncEngine";
import { emptyTimeline, markApprovalDecision, mergeHistoryAttachments, timelineFromEntries, type TimelineState } from "./timeline";
import type { HistoryEntry, RemoteSessionState, StreamEvent } from "./types";
export { compactionTerminalOf, COMPACTION_TERMINAL_TIMEOUT_MS } from "./timeline/useCompactionOutcomes";

const TIMELINE_LOAD_TIMEOUT_MS = 15_000;

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
  const compactingRef = useRef<Record<string, boolean>>({});
  const historyPagingRef = useRef<Record<string, HistoryPagingState>>({});
  const committedHistoryRef = useRef<Record<string, TimelineState["historyWindow"]>>({});
  const historyEpochRef = useRef(0);
  const { compactionWaitersRef, compactionTerminalsRef, cancelCompactionWaiters, awaitCompactionOutcome }
    = useCompactionOutcomes({ clientRef, selectedRef, syncEngineRef, historyEpochRef });
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
      const compactionTerminal = compactionTerminalOf(event);
      if (compactionTerminal) {
        // Settle before the visibility check below: a conversation switch during
        // compaction must not leave the initiator waiting for its outcome.
        const key = JSON.stringify([sid, compactionTerminal.operationId]);
        const waiter = compactionWaitersRef.current.get(key);
        if (waiter) {
          waiter.settle(compactionTerminal.outcome);
        } else {
          // Bound early/replayed terminals; keep multiple operations in a
          // session so an unrelated replay cannot overwrite a fast result.
          compactionTerminalsRef.current.set(key, compactionTerminal.outcome);
          if (compactionTerminalsRef.current.size > 32) {
            compactionTerminalsRef.current.delete(compactionTerminalsRef.current.keys().next().value!);
          }
        }
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
    [compactionTerminalsRef, compactionWaitersRef, reconcileSession, refreshModels, refreshSessions, selectedRef, setTitleOverrides],
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
      // A byte-trimmed tail page sheds whole oldest exchanges to fit the
      // bridge's reply budget, so it can come back holding fewer exchanges
      // than the window is meant to show (the tool-dense session that surfaced
      // this returned a single exchange). The shed exchanges are still durable
      // and still paged (hasMore stays true) — without backfilling them here
      // they only ever reappear if the user manually pages up, which reads as
      // "older messages were eaten". Backfill whole older exchanges until the
      // window holds its full exchange count or the journal runs out. Only a
      // page the bridge explicitly marked trimmed is short by construction; a
      // genuinely short session must not trigger these extra reads. The
      // backward byte budget only trims the newest page, so these gap reads
      // return complete exchanges.
      const countUsers = (rows: HistoryEntry[]) =>
        rows.reduce((n, row) => n + (row.role === "user" ? 1 : 0), 0);
      let tailTrimmed = response.trimmed === true;
      while (
        tailTrimmed &&
        hasMore &&
        countUsers(entries) < HISTORY_PAGE_USER_EXCHANGES &&
        nextBefore > 0
      ) {
        const older = await readHistoryPage(client, sessionId, nextBefore, isCurrent);
        const olderEntries = older.entries ?? [];
        const start = older.nextOffset ?? 0;
        if (
          !Number.isSafeInteger(start) || start < 0 || start >= nextBefore ||
          start + olderEntries.length !== nextBefore
        ) {
          throw new Error("history_tail_backfill_cursor_invalid");
        }
        entries = [...olderEntries, ...entries];
        nextBefore = start;
        hasMore = older.hasMore === true && start > 0;
        tailTrimmed = older.trimmed === true;
        if (olderEntries.length === 0) break;
      }
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
        // A cursor that does not advance (or points outside the requested
        // window) means the durable history shifted underneath the read —
        // the page cannot be trusted and the whole refresh must retry later.
        if (!Number.isSafeInteger(start) || start < 0 || start >= nextBefore) {
          throw new Error("history_gap_cursor_invalid");
        }
        if (start + olderEntries.length !== nextBefore) {
          // A gap page must end flush with the requested `before`. The one
          // legitimate exception is the desktop bridge's byte-budget trim
          // (prepare_backward_entries_page_with_cap): it sheds whole oldest
          // exchanges, advances the cursor past them, and always marks the
          // page hasMore. The same range is available untrimmed on request,
          // so bridge the gap from that re-read instead of failing the
          // refresh — failing it is what stranded older history: the
          // conversation collapsed to the tail and the paging cursor could
          // never rejoin it. A short page without hasMore is genuine journal
          // corruption: fail and retry.
          if (older.hasMore !== true) {
            throw new Error("history_gap_cursor_invalid");
          }
          const untrimmed = await readHistoryPage(client, sessionId, nextBefore, isCurrent, true);
          const untrimmedEntries = untrimmed.entries ?? [];
          const untrimmedStart = untrimmed.nextOffset ?? 0;
          if (!Number.isSafeInteger(untrimmedStart) || untrimmedStart < 0 ||
            untrimmedStart >= nextBefore ||
            untrimmedStart + untrimmedEntries.length !== nextBefore) {
            throw new Error("history_gap_cursor_invalid");
          }
          entries = [...untrimmedEntries, ...entries];
          nextBefore = untrimmedStart;
          hasMore = untrimmed.hasMore === true && untrimmedStart > 0;
          continue;
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
            // Exact joins are adjacent. A bridged gap needs no prefix
            // retention: the gap fill already folded those rows into the
            // fresh window.
            retained.endOffset === nextBefore,
          )
        : latest;
      // The retained cursor survives while the fresh window actually joined
      // the retained prefix into one contiguous window (an exact join, or a
      // gap bridged back over it). A fresh window that replaced the visible
      // conversation wholesale (restart on changed history) or that grew
      // past the retained page bottom hands paging over to its own cursor.
      const keepRetainedCursor =
        retained && history !== latest && nextBefore <= retained.endOffset;
      return {
        ...history,
        historyWindow: {
          nextBefore: keepRetainedCursor ? retained!.nextBefore : nextBefore,
          endOffset,
          hasMore: keepRetainedCursor
            ? retained!.hasMore && retained!.nextBefore > 0
            : hasMore,
        },
      };
    },
    [clientRef, selectedRef],
  );

  const loadOlderTimeline = useOlderHistory({ clientRef, selectedRef, syncEngineRef,
    historyPagingRef, setHistoryPaging, olderRequestRef });

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
      delete compactingRef.current[id];
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
        const waiting = [...compactionWaitersRef.current.values()].filter(waiter => waiter.sessionId === sessionId);
        const state = (
          await client.requestRetry<RemoteSessionState>(
            { type: "get_state", sessionId },
            sessionId,
          )
        ).data;
        if (clientRef.current === client && historyEpochRef.current === epoch && state.isCompacting === false) {
          // Only an explicit idle read started after registration can settle a
          // lost result. Projected flags/history previews are not evidence.
          for (const waiter of waiting) waiter.settle({ status: "unobserved" });
        }
        if (clientRef.current === client && selectedRef.current === sessionId
          && settingsRevisionRef.current === revision && settingsReadRef.current === read
          && historyEpochRef.current === epoch) {
          onSessionStateRef.current?.(sessionId, state);
        }
        return state;
      },
      requestHistory: loadHistory,
      // The lane the Desktop agreed to send decides what the integrity checks
      // may demand: a lean feed omits slices by design, so it must not be read
      // as loss (that reconciliation would retry forever). Read through the
      // ref, because the acked features land after the engine is built.
      feedOmitsIndices: () => clientRef.current?.feedsOmittedIndices() === true,
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
      compactingRef.current[commit.sessionId] = commit.timeline.compacting === true;
    });
    syncEngineRef.current = engine;
    return () => {
      unsubscribe();
      syncEngineRef.current = null;
      engine.clear();
    };
  }, [clientRef, compactionWaitersRef, loadHistory, selectedRef]);

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
      // When the terminal already arrived over a complete prefix the run is
      // whole, and re-reading it would show a sync notice for content that is
      // already rendered. The heal stays for the case it exists for: the
      // terminal never made it, so the timeline still reports streaming.
      if (run && engine.runCompleteLocally(sessionId, run)) return;
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
    compactingRef.current = {};
    cancelCompactionWaiters();
    historyPagingRef.current = {};
    committedHistoryRef.current = {};
    setHistoryPaging({});
  }, [cancelCompactionWaiters]);

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

  // Pull-to-refresh: rebuild the visible window from durable history. This is
  // the explicit escape hatch for any sync state the automatic reconciles
  // failed to heal — it re-runs the same open path, so a stuck or wrong
  // timeline converges to whatever the journal holds.
  const reloadTimeline = useCallback(() => {
    const sessionId = selectedRef.current;
    if (!sessionId) return;
    syncEngineRef.current?.restart(sessionId, "open");
  }, [selectedRef]);

  return {
    timeline,
    timelinePending,
    timelineError,
    timelineSyncStatus,
    canLoadOlderTimeline: selectedHistoryPaging?.hasMore ?? false,
    loadingOlderTimeline: selectedHistoryPaging?.loading ?? false,
    loadOlderTimeline,
    reloadTimeline,
    prepareTimelineOpen,
    syncEngineRef,
    streamingRef,
    compactingRef,
    hydrateAttachmentsRef,
    reconcileSession,
    handleEvent,
    applySessionStreaming,
    awaitCompactionOutcome,
    resetTimeline,
    ensureDraftTimeline,
    retryTimeline,
    // Test-only seam: lets controller tests seed a retained older-history
    // window without driving several real paging round trips.
    seedHistoryPaging: (sessionId: string, window: HistoryPagingState) => {
      historyPagingRef.current[sessionId] = window;
      setHistoryPaging((previous) => ({ ...previous, [sessionId]: window }));
    },
    historyPaging,
  };
}
