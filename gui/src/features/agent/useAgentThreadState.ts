import type { StoredRun, StoredThread } from "../../integrations/storage/threadStore";
import type { MessageAttachment } from "./agentThreadTypes";
import { useCallback, useEffect, useRef } from "react";
import { abortRun, getLatestRun } from "../../integrations/storage/threadStore";
import { usePolling } from "../../lib/usePolling";
import { matchesSettledRun } from "./agentMessageFormatters";
import { useRunReattach } from "./useRunReattach";
import { useSendMessage } from "./useSendMessage";
import { useThreadMessages } from "./useThreadMessages";

interface UseAgentThreadStateInput {
  thread: StoredThread | null;
  workspacePath?: string | null;
  loadingStore: boolean;
  modelId: string;
  thinkingLevel: string;
  pendingPrompt: { attachments?: MessageAttachment[]; id: string; content: string; targetThreadId: string } | null;
  onPromptConsumed: (id: string) => void;
  onThreadActivity: () => void;
}

// The run this thread is actively executing, or null. Guards on `threadId`
// because `recentRun` lags a thread switch by one poll — a stale run from the
// previous thread must not read as active here.
function activeRunIdOf(recentRun: StoredRun | null, threadId: string | null): string | null {
  return recentRun && recentRun.threadId === threadId && !matchesSettledRun(recentRun.status)
    ? recentRun.id
    : null;
}

export function useAgentThreadState({
  thread,
  workspacePath,
  loadingStore,
  modelId,
  thinkingLevel,
  pendingPrompt,
  onPromptConsumed,
  onThreadActivity,
}: UseAgentThreadStateInput) {
  const threadId = thread?.id ?? null;
  const workspaceId = thread?.workspaceId;
  const consumedPromptRef = useRef<string | null>(null);
  // True while a prompt is in flight for this thread. The agent rejects a second
  // concurrent prompt for the same session, so guard every send path with it.
  // Owned here because the send hook writes it and the re-attach hook reads it.
  const sendingRef = useRef(false);

  const {
    loadingThread,
    loadingIndicator,
    messages,
    recentRun,
    renderWorkspace,
    reloadMessagesQuiet,
    refreshRecentRun,
    setMessages,
    setRecentRun,
    messagesGenRef,
  } = useThreadMessages({ threadId, workspaceId, workspacePath, agentSessionId: thread?.agentSessionId });

  // The run this thread is currently executing, if any. Runs stream server-side
  // and persist their events regardless of which thread is in the foreground, so
  // this is the anchor for re-attaching a live preview to a conversation that was
  // started, backgrounded, and returned to (or picked up after an app reload) —
  // the in-flight send only drives the view while its own thread stays
  // foreground. Guarded on `threadId` because `recentRun` lags a thread switch by
  // one poll, and a stale run from the previous thread must not leak into this
  // one. Null once the run settles.
  const activeRunId = activeRunIdOf(recentRun, threadId);
  // Epoch-ms anchor for the live elapsed timer of a re-attached run. Stable while
  // the run stays active (derived from persisted run times), so it doesn't churn
  // the resume effect the way the `recentRun` object identity would.
  const activeRunStartedAt = activeRunId ? (recentRun?.startedAt ?? recentRun?.createdAt ?? null) : null;

  const { handleSend, abandonSend } = useSendMessage({
    thread,
    threadId,
    modelId,
    thinkingLevel,
    activeRunId,
    sendingRef,
    setMessages,
    setRecentRun,
    refreshRecentRun,
    onThreadActivity,
  });

  useRunReattach({
    threadId,
    workspaceId,
    activeRunId,
    activeRunStartedAt,
    sendingRef,
    setMessages,
    refreshRecentRun,
    reloadMessagesQuiet,
    messagesGenRef,
  });

  // ── Settle watchdog ────────────────────────────────────────────────────
  // While a local send owns the view, its pipeline is the only path that
  // finalizes the run. If that pipeline's invoke never resolves (window
  // hidden → macOS suspends the webview and the response is never applied),
  // the pipeline hangs forever: the composer locked on `sendingRef`, the
  // bubble frozen mid-stream. The backend settles the run row itself the
  // instant the stream ends, so the row is the truth: once it is terminal,
  // abandon the hung send (release the lock, disconnect the late pipeline
  // from this view) and force-reload the persisted reply. Runs periodically
  // and immediately when the window becomes visible/focused again, so a run
  // that finished while hidden unsticks the moment the user returns.
  const threadIdRef = useRef(threadId);
  threadIdRef.current = threadId;
  const reconcileHungSend = useCallback(async () => {
    const targetThreadId = threadIdRef.current;
    if (!targetThreadId || !sendingRef.current)
      return;
    const latest = await getLatestRun(targetThreadId).catch(() => null);
    // Re-check ownership after the await: a thread switch may have handed
    // `sendingRef` to a new send, which a stale result must not abandon.
    if (
      !latest
      || latest.threadId !== targetThreadId
      || threadIdRef.current !== targetThreadId
      || !sendingRef.current
      || !matchesSettledRun(latest.status)
    ) {
      return;
    }
    abandonSend();
    setRecentRun(latest);
    void reloadMessagesQuiet(targetThreadId, true);
  }, [abandonSend, reloadMessagesQuiet, sendingRef, setRecentRun]);

  usePolling(() => {
    void reconcileHungSend();
  }, 15_000, {
    enabled: Boolean(threadId),
    deps: [reconcileHungSend],
  });

  useEffect(() => {
    const onVisible = () => {
      if (document.visibilityState === "visible")
        void reconcileHungSend();
    };
    document.addEventListener("visibilitychange", onVisible);
    window.addEventListener("focus", onVisible);
    return () => {
      document.removeEventListener("visibilitychange", onVisible);
      window.removeEventListener("focus", onVisible);
    };
  }, [reconcileHungSend]);

  // Interrupt the in-flight run for this thread. Best-effort: the backend stops
  // the agent and marks the run `cancelled`; the in-flight send then settles the
  // streaming bubble to the partial reply (see the cancelled branch in the send
  // pipeline), and refreshing `recentRun` clears `activeRunId` so the resume
  // effect reconciles. Safe to call when nothing is running (resolves to a no-op).
  const handleAbort = useCallback(async () => {
    if (!threadId)
      return;
    const runId
      = activeRunIdOf(recentRun, threadId)
        ?? messages.find(message => message.role === "assistant" && message.status === "streaming")?.runId ?? null;
    if (!runId)
      return;
    try {
      await abortRun({ threadId, runId });
    }
    catch {
      // The run may already have finished; the refresh below still reconciles.
    }
    await refreshRecentRun(threadId, workspaceId);
    onThreadActivity();
  }, [messages, onThreadActivity, recentRun, refreshRecentRun, workspaceId, threadId]);

  useEffect(() => {
    if (!thread || loadingThread || loadingStore || !pendingPrompt)
      return;
    if (consumedPromptRef.current === pendingPrompt.id)
      return;
    // Only deliver the prompt to the thread it was composed for. A fast thread
    // switch during the (async) message load can make `thread` the newly-opened
    // conversation while this prompt still targets the one just created — sending
    // here would drop the first message (and its attachments) into the wrong
    // chat and persist it there. Wait for the target thread to be active.
    if (pendingPrompt.targetThreadId !== thread.id)
      return;

    consumedPromptRef.current = pendingPrompt.id;
    onPromptConsumed(pendingPrompt.id);
    void handleSend({ attachments: pendingPrompt.attachments ?? [], content: pendingPrompt.content });
  }, [handleSend, loadingStore, loadingThread, onPromptConsumed, pendingPrompt, thread]);

  return {
    handleAbort,
    handleSend,
    loadingThread,
    loadingIndicator,
    messages,
    recentRun,
    renderWorkspace,
  };
}
