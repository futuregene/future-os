import type { StoredRun } from "../../integrations/storage/threadStore";
import type { AgentMessage } from "./agentThreadTypes";
import { useCallback, useEffect, useRef, useState } from "react";
import i18n from "../../i18n";
import { listMessages, listRuns } from "../../integrations/storage/threadStore";
import { errorMessage } from "../../lib/errors";
import { upsertFutureReferenceData } from "../markdown/futureReferenceStore";
import { matchesSettledRun, toAgentMessage } from "./agentMessageFormatters";
import { restoreMessageActivities } from "./threadRunProjection";

interface UseThreadMessagesInput {
  threadId: string | null;
  workspaceId?: string | null;
}

/**
 * Owns a thread's message list + recent-run status: loads/restores messages on
 * thread switch, keeps a live run polling while one is active, and exposes the
 * quiet reload used to swap a synthetic streaming bubble for the persisted
 * assistant message once a background run settles.
 */
export function useThreadMessages({ threadId, workspaceId }: UseThreadMessagesInput) {
  const [messages, setMessages] = useState<AgentMessage[]>([]);
  const [loadingThread, setLoadingThread] = useState(true);
  const [recentRun, setRecentRun] = useState<StoredRun | null>(null);

  // Tracks the thread this view currently shows. Since AgentThread is not keyed
  // by threadId (it stays mounted across thread switches), an async write from a
  // background reload must verify its target is still active before touching
  // state — otherwise a slow load for thread A can overwrite thread B's view.
  const activeThreadIdRef = useRef(threadId);
  activeThreadIdRef.current = threadId;

  // Guard against overlapping refreshes (poll tick, send, thread switch) where a
  // slow response lands after a newer one and writes stale run state — e.g. a
  // previous thread's run after switching. Newest call wins.
  const recentRunGenRef = useRef(0);
  const refreshRecentRun = useCallback(async (targetThreadId: string, targetWorkspaceId?: string | null) => {
    const generation = ++recentRunGenRef.current;
    try {
      const runs = await listRuns(targetThreadId);
      if (generation !== recentRunGenRef.current) {
        return;
      }
      const latestRun = runs[0] ?? null;
      setRecentRun(latestRun);
      if (latestRun) {
        upsertFutureReferenceData(targetWorkspaceId, "run", latestRun.id, latestRun);
      }
    }
    catch {
      // Run-status refresh is best-effort: a failure here must not blank the
      // thread (it runs alongside listMessages in loadThreadMessages via
      // Promise.all) or abort an in-flight send. Keep the previous recentRun
      // until the next poll. The waiting-approval prompt is rendered separately
      // by AgentThread from `activeApproval`, so no message rewrite is needed.
    }
  }, []);

  // Reload a thread's messages from the store without flipping the full-screen
  // loading state — used to swap a synthetic streaming bubble for the persisted
  // assistant message once a background run settles.
  const reloadMessagesQuiet = useCallback(async (targetThreadId: string) => {
    try {
      const storedMessages = await listMessages(targetThreadId);
      const agentMessages = storedMessages.map(toAgentMessage);
      const restoredMessages = await restoreMessageActivities(agentMessages, targetThreadId);
      // Drop the result if the user switched threads while this was in flight —
      // writing it now would paint the old thread's messages into the new view.
      if (targetThreadId !== activeThreadIdRef.current) {
        return;
      }
      setMessages(restoredMessages);
    }
    catch {
      // Best-effort refresh: keep the current messages on failure.
    }
  }, []);

  useEffect(() => {
    // Hand-rolled cancel guard (not useAsyncResource): loads messages, refreshes
    // the recent run, and restores activities into several states at once, which
    // doesn't map onto the primitive's single-resource shape. See gui/CLAUDE.md §4.
    let cancelled = false;

    async function loadThreadMessages() {
      if (!threadId) {
        setMessages([]);
        setLoadingThread(false);
        return;
      }

      setLoadingThread(true);
      try {
        const [storedMessages] = await Promise.all([listMessages(threadId), refreshRecentRun(threadId, workspaceId)]);
        const agentMessages = storedMessages.map(toAgentMessage);
        const restoredMessages = await restoreMessageActivities(agentMessages, threadId);
        if (!cancelled) {
          setMessages(restoredMessages);
        }
      }
      catch (error) {
        const message = errorMessage(error);
        if (!cancelled) {
          setMessages([
            {
              id: "store_error",
              role: "assistant",
              author: i18n.t("agent:author.system"),
              authorKey: "author.system",
              content: i18n.t("agent:thread.messagesLoadFailed", { message }),
              createdAt: new Date().toISOString(),
            },
          ]);
        }
      }
      finally {
        if (!cancelled) {
          setLoadingThread(false);
        }
      }
    }

    void loadThreadMessages();

    return () => {
      cancelled = true;
    };
  }, [refreshRecentRun, workspaceId, threadId]);

  // Stable flag so the poll effect keys on "is a run active", not on the
  // recentRun object identity — refreshRecentRun replaces recentRun with a fresh
  // object every tick, which would otherwise tear down and rebuild the interval
  // (and re-render) each 1.5s.
  const isRunActive = Boolean(recentRun && !matchesSettledRun(recentRun.status));

  useEffect(() => {
    if (!threadId || !isRunActive)
      return;

    const timer = window.setInterval(() => {
      void refreshRecentRun(threadId, workspaceId);
    }, 1500);

    return () => window.clearInterval(timer);
  }, [isRunActive, refreshRecentRun, workspaceId, threadId]);

  return {
    loadingThread,
    messages,
    recentRun,
    reloadMessagesQuiet,
    refreshRecentRun,
    setMessages,
    setRecentRun,
  };
}
