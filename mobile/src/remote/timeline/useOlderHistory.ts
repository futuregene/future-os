import { HISTORY_PAGE_TIMEOUT_MS, HISTORY_PAGE_USER_EXCHANGES, commitHistoryPage, diagnosticError } from "./historyPaging";
import type { HistoryPagingState } from "./historyPaging";
import type { Dispatch, MutableRefObject, SetStateAction } from "react";
import { useCallback } from "react";
import type { RemoteClient } from "../client";
import { requestReadPage } from "../readPages";
import { SyncEngine } from "../syncEngine";
import { timelineFromEntries } from "../timeline";
import type { EntriesData } from "../types";
interface Options {
  clientRef: MutableRefObject<RemoteClient | null>;
  selectedRef: MutableRefObject<string>;
  syncEngineRef: MutableRefObject<SyncEngine | null>;
  historyPagingRef: MutableRefObject<Record<string, HistoryPagingState>>;
  setHistoryPaging: Dispatch<SetStateAction<Record<string, HistoryPagingState>>>;
  olderRequestRef: MutableRefObject<{ sessionId: string; controller: AbortController; } | null>;
}

export function useOlderHistory({ clientRef, selectedRef, syncEngineRef, historyPagingRef, setHistoryPaging, olderRequestRef }: Options) {
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
  }, [clientRef, historyPagingRef, olderRequestRef, selectedRef, setHistoryPaging, syncEngineRef]);

  return loadOlderTimeline;
}
