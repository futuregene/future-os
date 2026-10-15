import { HISTORY_PAGE_TIMEOUT_MS, HISTORY_PAGE_USER_EXCHANGES, HistoryPageHiddenError, commitHistoryPage, diagnosticError } from "./historyPaging";
import type { HistoryPagingState } from "./historyPaging";
import type { Dispatch, MutableRefObject, SetStateAction } from "react";
import { useCallback } from "react";
import type { RemoteClient } from "../client";
import { requestReadPage } from "../readPages";
import { SyncEngine } from "../syncEngine";
import { timelineFromEntries } from "../timeline";
import type { EntriesData } from "../types";

/**
 * A downloaded page whose commit could not even start because the lane went
 * dark (backgrounded or restarted). It never reached the timeline, so it is
 * safe to re-offer unchanged once the lane can publish again — replaying it
 * is not a second copy, it is the first landing of the same rows.
 */
type PendingOlderPage = {
  sessionId: string;
  before: number;
  nextBefore: number;
  endOffset: number;
  hasMore: boolean;
  older: ReturnType<typeof timelineFromEntries>;
};

export type OlderFailureMarker = (sessionId: string, failed: boolean) => void;

interface Options {
  clientRef: MutableRefObject<RemoteClient | null>;
  selectedRef: MutableRefObject<string>;
  syncEngineRef: MutableRefObject<SyncEngine | null>;
  historyPagingRef: MutableRefObject<Record<string, HistoryPagingState>>;
  setHistoryPaging: Dispatch<SetStateAction<Record<string, HistoryPagingState>>>;
  olderRequestRef: MutableRefObject<{ sessionId: string; controller: AbortController; } | null>;
  markOlderFailure: OlderFailureMarker;
}

export type OlderPageResult =
  | false
  | { kind: "committed"; ids: string[] }
  | { kind: "hidden"; pending: PendingOlderPage };

export function useOlderHistory({ clientRef, selectedRef, syncEngineRef, historyPagingRef, setHistoryPaging, olderRequestRef, markOlderFailure }: Options) {

  const loadOlderTimeline = useCallback(async (): Promise<OlderPageResult> => {
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
    markOlderFailure(sessionId, false);
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
    let onAbort = () => { };
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
      try {
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
      } catch (error) {
        // The lane dropped the mutation before it could run (app backgrounded,
        // or the lane restarted under the queued op). The page never touched
        // the timeline, so it is not a failed page: keep the cursor and hand
        // the caller the downloaded page to re-offer once the lane can commit
        // again, rather than showing a failure for a state the user caused
        // and can undo by returning to the conversation.
        if (error instanceof HistoryPageHiddenError && isCurrent()) {
          const pending: PendingOlderPage = {
            sessionId,
            before: current.nextBefore,
            nextBefore,
            endOffset: current.endOffset,
            hasMore: response.data.hasMore === true && nextBefore > 0,
            older,
          };
          return { kind: "hidden", pending };
        }
        throw error;
      }
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
      return { kind: "committed", ids: older.items.map((item) => item.id) };
    } catch (error) {
      if (historyPagingRef.current[sessionId] !== loading) return false;
      const failed = { ...current, loading: false };
      historyPagingRef.current[sessionId] = failed;
      setHistoryPaging((previous) => ({ ...previous, [sessionId]: failed }));
      markOlderFailure(sessionId, true);
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
  }, [clientRef, historyPagingRef, olderRequestRef, markOlderFailure, selectedRef, setHistoryPaging, syncEngineRef]);

  return loadOlderTimeline;
}

export type { PendingOlderPage };
