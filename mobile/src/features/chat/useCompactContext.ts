import { useCallback, useRef } from "react";
import type { TFunction } from "i18next";
import { useRemote } from "../../remote/RemoteContext";
import { showToast } from "./utils";

type Remote = ReturnType<typeof useRemote>;

export interface CompactContextApi {
  compact: () => Promise<void>;
}

/**
 * Manual context compaction from the phone.
 *
 * The composer already renders the operation itself (a running divider, then a
 * committed or failed one), so this hook adds only what that divider cannot
 * say: a rejected request, a no-op compaction that leaves no marker at all, and
 * a missing result.
 */
export function useCompactContext(remote: Remote, t: TFunction): CompactContextApi {
  const { compactContext, awaitCompactionOutcome, compacting } = remote;
  // The request itself is millisecond-scale; the operation is tracked by the
  // session's own compacting state, not by this flag.
  const requestInFlight = useRef(false);
  const compact = useCallback(async () => {
    if (requestInFlight.current) return;
    // Gate on the *authoritative* compacting state rather than on this client's
    // own bookkeeping: a terminal event that never reached the phone must not
    // leave the action silently refusing every later tap. A duplicate that
    // slips through the acknowledgement window comes back as the Agent's own
    // rejection, which is a truthful message rather than silence.
    if (compacting) {
      showToast(t("chat.compacting"));
      return;
    }
    requestInFlight.current = true;
    try {
      const { sessionId, operationId } = await compactContext();
      const outcome = await awaitCompactionOutcome(sessionId, operationId);
      if (outcome.status === "failed") {
        showToast(t("chat.compactionRequestFailed", {
          message: outcome.error || t("failure.unknown"),
        }));
      }
      else if (outcome.status === "unchanged") {
        showToast(t(outcome.alreadyCompacted || outcome.reused
          ? "chat.compactionNoNewContent"
          : "chat.compactionNotNeeded"));
      }
      else if (outcome.status === "timeout") {
        showToast(t("chat.compactionWaitTimedOut"));
      }
      // "committed" needs no toast (the divider reported it), and "unobserved"
      // has no result to report: the session stopped compacting without this
      // client seeing the terminal frame, so say nothing rather than invent one.
    }
    catch (error) {
      showToast(t("chat.compactionRequestFailed", {
        message: error instanceof Error ? error.message : String(error),
      }));
    }
    finally {
      requestInFlight.current = false;
    }
  }, [awaitCompactionOutcome, compactContext, compacting, t]);

  return { compact };
}
