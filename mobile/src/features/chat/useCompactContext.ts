import { useCallback, useLayoutEffect, useRef, useState } from "react";
import type { TFunction } from "i18next";
import type { useRemote } from "../../remote/RemoteContext";
import { showToast } from "./utils";

type Remote = ReturnType<typeof useRemote>;
type Attempt = { scope: string; controller: AbortController };

export interface CompactContextApi {
  compact: () => Promise<void>;
  /** Immediate local admission/wait feedback, before the first Agent event. */
  pending: boolean;
}

export function useCompactContext(remote: Remote, t: TFunction): CompactContextApi {
  const { compactContext, awaitCompactionOutcome, compacting, busy, draft, selectedSessionId } = remote;
  const streaming = remote.timeline.streaming;
  const supported = remote.capabilities.has("compaction_v1");
  const scope = JSON.stringify([remote.credentials?.pairId, remote.presence?.bridgeInstanceId, selectedSessionId]);
  const attemptRef = useRef<Attempt | null>(null);
  const [attempt, setAttempt] = useState<Attempt | null>(null);
  useLayoutEffect(() => () => {
    const active = attemptRef.current;
    if (active?.scope === scope) {
      active.controller.abort();
      attemptRef.current = null;
    }
  }, [scope]);

  const compact = useCallback(async () => {
    if (attemptRef.current?.scope === scope || compacting) return;
    if (!supported || draft || !selectedSessionId || streaming || busy) return;
    attemptRef.current?.controller.abort();
    const current = { scope, controller: new AbortController() };
    attemptRef.current = current;
    setAttempt(current); // render the spinner/disable sends before awaiting the RPC
    const isCurrent = () => !current.controller.signal.aborted && attemptRef.current === current;
    try {
      const acknowledgement = await compactContext();
      if (!isCurrent()) return;
      if (acknowledgement.sessionId !== selectedSessionId) throw new Error("compaction_session_changed");
      const outcome = await awaitCompactionOutcome(
        acknowledgement.sessionId, acknowledgement.operationId, undefined, current.controller.signal,
      );
      if (!isCurrent()) return;
      if (outcome.status === "failed") {
        showToast(t("chat.compactionRequestFailed", { message: outcome.error || t("failure.unknown") }));
      } else if (outcome.status === "unchanged") {
        showToast(t(outcome.alreadyCompacted || outcome.reused ? "chat.compactionNoNewContent" : "chat.compactionNotNeeded"));
      } else if (outcome.status === "timeout") {
        showToast(t("chat.compactionWaitTimedOut"));
      }
      // committed is visible in history; unobserved/cancelled cannot claim success.
    } catch (error) {
      if (isCurrent()) showToast(t("chat.compactionRequestFailed", {
        message: error instanceof Error ? error.message : String(error),
      }));
    } finally {
      if (attemptRef.current === current) {
        attemptRef.current = null;
        setAttempt(null);
      }
    }
  }, [awaitCompactionOutcome, compactContext, compacting, streaming, busy, draft, selectedSessionId, supported, scope, t]);

  return { compact, pending: attempt?.scope === scope && !attempt.controller.signal.aborted };
}
