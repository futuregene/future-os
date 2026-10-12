import type { MutableRefObject } from "react";
import { useCallback, useEffect, useRef } from "react";
import { AppState } from "react-native";
import type { RemoteClient } from "../client";
import { SyncEngine } from "../syncEngine";
import type { RemoteSessionState, StreamEvent, CompactionOutcome } from "../types";

export const COMPACTION_TERMINAL_TIMEOUT_MS = 30 * 60 * 1000;
/**
 * The terminal compaction event behind an operation id.
 *
 * `compaction_end` is deliberately absent: it is the released-journal
 * compatibility marker and carries no operation id from the async path.
 */
export function compactionTerminalOf(
  event: StreamEvent,
): { operationId: string; outcome: CompactionOutcome; } | null {
  if (
    event.type !== "compaction_committed"
    && event.type !== "compaction_failed"
    && event.type !== "compaction_unchanged"
  )
    return null;
  let parsed: unknown;
  try {
    parsed = JSON.parse(event.data);
  } catch {
    return null;
  }
  const payload = (parsed && typeof parsed === "object" ? parsed : {}) as Record<string, unknown>;
  const operationId = typeof payload.operation_id === "string" ? payload.operation_id : "";
  if (!operationId) return null;
  if (event.type === "compaction_failed") {
    return {
      operationId,
      outcome: {
        status: "failed",
        ...(typeof payload.error === "string" ? { error: payload.error } : {}),
      },
    };
  }
  if (event.type === "compaction_unchanged") {
    return {
      operationId,
      outcome: {
        status: "unchanged",
        alreadyCompacted: payload.already_compacted === true,
        reused: payload.reused === true,
      },
    };
  }
  return { operationId, outcome: { status: "committed" } };
}
interface Options {
  clientRef: MutableRefObject<RemoteClient | null>;
  selectedRef: MutableRefObject<string>;
  syncEngineRef: MutableRefObject<SyncEngine | null>;
  historyEpochRef: MutableRefObject<number>;
}
export function useCompactionOutcomes({ clientRef, selectedRef, syncEngineRef, historyEpochRef }: Options) {
  // Manual compaction is asynchronous: the request returns before the summary
  // runs, so the waiter and the event race in both directions. Waiters are
  // keyed by operation id (a replayed older operation can never settle a new
  // request), and a terminal event that arrives first is buffered per session
  // until its initiator correlates it.
  const compactionWaitersRef = useRef<
    Map<string, { sessionId: string; promise: Promise<CompactionOutcome>; settle: (outcome: CompactionOutcome) => void; }>
  >(new Map());
  const compactionTerminalsRef = useRef<Map<string, CompactionOutcome>>(new Map());
  const cancelCompactionWaiters = useCallback(() => {
    for (const waiter of [...compactionWaitersRef.current.values()]) waiter.settle({ status: "cancelled" });
    compactionTerminalsRef.current.clear();
  }, []);
  useEffect(() => cancelCompactionWaiters, [cancelCompactionWaiters]);
  /**
   * Await the terminal event of the compaction behind `operationId`.
   *
   * A timeout is reported as `timeout`, not `failed`: an unobserved terminal
   * event (backgrounded app, dropped stream, long summary) proves nothing
   * about the operation itself.
   */
  const awaitCompactionOutcome = useCallback(
    (
      sessionId: string,
      operationId: string,
      timeoutMs: number = COMPACTION_TERMINAL_TIMEOUT_MS,
      signal?: AbortSignal,
    ): Promise<CompactionOutcome> => {
      if (signal?.aborted) return Promise.resolve({ status: "cancelled" });
      const key = JSON.stringify([sessionId, operationId]);
      const buffered = compactionTerminalsRef.current.get(key);
      if (buffered) {
        compactionTerminalsRef.current.delete(key);
        return Promise.resolve(buffered);
      }
      const existing = compactionWaitersRef.current.get(key);
      if (existing) return existing.promise;
      let resolve!: (outcome: CompactionOutcome) => void;
      const promise = new Promise<CompactionOutcome>(done => { resolve = done; });
      let finished = false;
      let pollTimer: ReturnType<typeof setTimeout> | undefined;
      const settle = (outcome: CompactionOutcome) => {
        if (finished) return;
        finished = true;
        clearTimeout(timer);
        clearTimeout(pollTimer);
        signal?.removeEventListener("abort", cancel);
        compactionWaitersRef.current.delete(key);
        resolve(outcome);
      };
      const cancel = () => settle({ status: "cancelled" });
      const timer = setTimeout(() => settle({ status: "timeout" }), timeoutMs);
      compactionWaitersRef.current.set(key, { sessionId, promise, settle });
      signal?.addEventListener("abort", cancel, { once: true });
      // Recover lost started AND terminal frames while staying on this screen.
      // One bounded, read-only probe at a time; never retry the write operation.
      const epoch = historyEpochRef.current;
      const probe = async () => {
        const client = clientRef.current;
        if (finished) return;
        if (client && selectedRef.current === sessionId && AppState.currentState !== "background") {
          try {
            const { data } = await client.requestRetry<RemoteSessionState>({ type: "get_state", sessionId }, sessionId);
            if (!finished && clientRef.current === client && historyEpochRef.current === epoch && data.isCompacting === false) {
              settle({ status: "unobserved" });
              syncEngineRef.current?.reconcile(sessionId, "resend");
            }
          } catch { /* Unreachable is not idle; keep waiting within the deadline. */ }
        }
        if (!finished) pollTimer = setTimeout(() => { void probe(); }, 5_000);
      };
      pollTimer = setTimeout(() => { void probe(); }, 5_000);
      return promise;
    },
    [clientRef, historyEpochRef, selectedRef, syncEngineRef],
  );

  return { compactionWaitersRef, compactionTerminalsRef, cancelCompactionWaiters, awaitCompactionOutcome };
}
