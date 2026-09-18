import { useCallback, useState } from "react";
import type { TFunction } from "i18next";
import { useRemote } from "../../remote/RemoteContext";
import { showToast } from "./utils";

type Remote = ReturnType<typeof useRemote>;

export interface CompactContextApi {
  compact: () => Promise<void>;
  /** True while this client waits for its own terminal compaction event. */
  pending: boolean;
}

/**
 * Manual context compaction from the phone.
 *
 * The composer already renders the operation itself (a running divider, then a
 * committed or failed one), so this hook adds only what that divider cannot
 * say: a rejected request, a no-op compaction that leaves no marker at all,
 * and a missing result.
 */
export function useCompactContext(remote: Remote, t: TFunction): CompactContextApi {
  const [pending, setPending] = useState(false);
  const { compactContext, awaitCompactionOutcome, compacting } = remote;
  const compact = useCallback(async () => {
    // One manual compaction at a time. The Agent would reject a second request
    // anyway; refusing here keeps a double tap from reporting a scary error.
    if (pending || compacting) return;
    setPending(true);
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
    }
    catch (error) {
      showToast(t("chat.compactionRequestFailed", {
        message: error instanceof Error ? error.message : String(error),
      }));
    }
    finally {
      setPending(false);
    }
  }, [awaitCompactionOutcome, compactContext, compacting, pending, t]);

  return { compact, pending };
}
