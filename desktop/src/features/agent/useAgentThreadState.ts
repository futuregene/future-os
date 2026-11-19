import type { MessageAttachment } from "@future-os/thread-projection";
import type {
  StoredRun,
  StoredThread,
} from "../../integrations/storage/threadStore";
import { matchesSettledRun } from "@future-os/thread-projection";
import { useCallback, useEffect, useRef } from "react";
import { abortRun, getRun } from "../../integrations/storage/threadStore";
import { useCommittedRef } from "../../lib/useCommittedRef";
import { usePolling } from "../../lib/usePolling";
import { restoreComposerDraft } from "./composerDraft";
import {
  clearPromptSendInFlight,
  isPromptSendInFlight,
  markPromptSendInFlight,
} from "./pendingPromptSends";
import { useRunReattach } from "./useRunReattach";
import { useQueuedSend, useSendMessage } from "./useSendMessage";
import { useThreadMessages } from "./useThreadMessages";

interface UseAgentThreadStateInput {
  thread: StoredThread | null;
  workspacePath?: string | null;
  loadingStore: boolean;
  modelId: string;
  thinkingLevel: string;
  /**
   * Whether the Agent reports a run in flight for this conversation — the
   * shell's one-second answer (`thread-streaming-updated`), which is the same
   * signal the rail files rows by.
   *
   * `false` is the one value that does anything: it says "nothing is streaming
   * here", which is an invitation to check the run row, not a verdict (see
   * `settleStaleReply`). `undefined` means the shell did not say, and nothing
   * is reconciled on that signal alone.
   */
  streaming?: boolean;
  pendingPrompt: {
    attachments?: MessageAttachment[];
    id: string;
    content: string;
    targetThreadId: string;
  } | null;
  onPromptConsumed: (id: string) => void;
  onThreadActivity: () => void;
}

// The run this thread is actively EXECUTING, or null.
//
// A run the Agent accepted but holds queued is deliberately not it. That row
// exists from the moment the message is written, and the Agent may fold it into
// the run ahead so it never executes at all — anchoring the live preview on it
// would watch a run with no events (no streaming) and keep failing against a
// run id the Agent does not know ("sync paused"). The queue panel reports
// those rows; the live lane belongs to whatever is running.
function activeRunIdOf(recentRun: StoredRun | null): string | null {
  return recentRun
    && !matchesSettledRun(recentRun.status)
    && recentRun.status !== "queued"
    ? recentRun.id
    : null;
}

export function useAgentThreadState({
  thread,
  workspacePath,
  loadingStore,
  modelId,
  thinkingLevel,
  streaming,
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
    hasOlderHistory,
    historyTruncated,
    loadOlderHistory,
    loadAllHistoryForSearch,
    historyError,
    historyStatus,
    sessionChanged,
    messages,
    recentRun,
    renderWorkspace,
    reloadThreadHistory,
    refreshRecentRun,
    setMessages,
    setRecentRun,
  } = useThreadMessages({
    threadId,
    workspaceId,
    workspacePath,
    agentSessionId: thread?.agentSessionId,
  });

  // Mirror the message list so handleAbort can read the latest streaming
  // bubble without depending on `messages` — the array changes identity on
  // every streaming push, and handleAbort feeds the memoized Composer, so a
  // fresh identity per push would defeat the memo.
  const messagesRef = useCommittedRef(messages);

  // The run this thread is currently executing, if any. Runs stream server-side
  // and persist their events regardless of which thread is in the foreground, so
  // this is the anchor for re-attaching a live preview to a conversation that was
  // started, backgrounded, and returned to (or picked up after an app reload) —
  // the in-flight send only drives the view while its own thread stays
  // foreground. Null once the run settles.
  const activeRunId = activeRunIdOf(recentRun);
  // Epoch-ms anchor for the live elapsed timer of a re-attached run. Stable while
  // the run stays active (derived from persisted run times), so it doesn't churn
  // the resume effect the way the `recentRun` object identity would.
  const activeRunStartedAt = activeRunId
    ? (recentRun?.startedAt ?? recentRun?.createdAt ?? null)
    : null;

  const { handleSend, abandonSend, localSendRef } = useSendMessage({
    thread,
    modelId,
    thinkingLevel,
    activeRunId,
    sendingRef,
    setMessages,
    setRecentRun,
    refreshRecentRun,
    onThreadActivity,
  });
  // The message a reader writes while the reply still streams: it leaves for the
  // Agent's own queue instead of waiting in this window (`useQueuedSend`).
  const handleQueuedSend = useQueuedSend({
    thread,
    modelId,
    thinkingLevel,
    setMessages,
    refreshRecentRun,
    onThreadActivity,
  });

  // A new owner invalidates the old send pipeline; first binding keeps its owner.
  const writerRef = useRef(setMessages);
  useEffect(() => {
    if (writerRef.current !== setMessages) {
      writerRef.current = setMessages;
      abandonSend();
    }
  }, [setMessages, abandonSend]);

  useRunReattach({
    threadId,
    workspaceId,
    activeRunId,
    activeRunStartedAt,
    loadingThread,
    sendingRef,
    setMessages,
    refreshRecentRun,
    reloadThreadHistory,
  });

  // ── Settle reconciliation ──────────────────────────────────────────────
  // A reply is drawn live — timer ticking, stop button, "thinking" — while
  // this view owns its send (`sendingRef`) or is re-attached to the run
  // (`activeRunId`). Only the terminal push takes it out of that state, and a
  // push can be lost: the invoke never resolves (window hidden → macOS
  // suspends the webview and the response is never applied), a coalesce drop,
  // a backend restart. The bubble then stays "streaming" with nobody driving
  // it, and the composer stays locked.
  //
  // The run row is the truth in every one of those cases: the backend settles
  // it itself the instant the stream ends. So this re-reads it and, when it is
  // terminal, abandons the send (releasing the lock and disconnecting the late
  // pipeline) and force-reloads the persisted reply.
  //
  // The read, not the trigger, decides — and the trigger is deliberately cheap:
  // `false` from the shell's one-second streaming answer only says "worth
  // asking". An empty answer is not evidence (`reconcile_streaming_observers`
  // returns an empty set on any Agent RPC error), and settling a live run on it
  // would freeze the reply as complete with no stop button while the run keeps
  // billing (`applyRunMetadata` has the same warning).
  const settleStaleReply = useCallback(async () => {
    if (!threadId || writerRef.current !== setMessages)
      return;
    // While a send owns the view it is the only path allowed to finalize its own
    // run, so that run is the one to read — never a bubble an earlier run left.
    const localSend = sendingRef.current ? localSendRef.current : null;
    const runId = sendingRef.current ? localSend?.runId ?? null : activeRunId;
    if (!runId)
      return;
    const latest = await getRun(runId).catch(() => null);
    // Re-check after the await: the send may have settled, been abandoned, or
    // been replaced while the read was in flight.
    if (
      !latest
      || latest.id !== runId
      || latest.threadId !== threadId
      || writerRef.current !== setMessages
      || !matchesSettledRun(latest.status)
      || (sendingRef.current
        ? localSendRef.current !== localSend
        : activeRunId !== runId)
    ) {
      return;
    }
    if (localSend)
      abandonSend();
    setRecentRun(latest);
    // A re-attached run has its own settle effect: clearing `activeRunId`
    // reloads the persisted reply (`useRunReattach`). An abandoned send has no
    // such effect, so its reload is issued here.
    if (localSend)
      void reloadThreadHistory(threadId, true);
  }, [
    abandonSend,
    activeRunId,
    localSendRef,
    reloadThreadHistory,
    sendingRef,
    setMessages,
    setRecentRun,
    threadId,
  ]);

  // The shell's one-second answer: is a run in flight for this conversation?
  //
  // This is the same signal the rail files rows by, and reading it is what keeps
  // the two from disagreeing. The rail moved a conversation to "Earlier" the
  // second its run ended (the agent's streaming set drops it within a second),
  // while this view held a live timer, "thinking" and a stop button on that same
  // reply until a 15–30 second fallback noticed: measured 2026-11-19, the
  // transcript was still ticking 47 seconds after its run had settled, next to a
  // sidebar that already said the conversation was done. A false answer here
  // closes that window to about a second.
  useEffect(() => {
    if (streaming !== false)
      return;
    void settleStaleReply();
  }, [settleStaleReply, streaming]);

  // Backstops for the same read, for the signals a push can also lose: a
  // low-frequency pass while the view is open, and an immediate pass when the
  // window becomes visible/focused again (a run that finished while hidden
  // unsticks the moment the user returns).
  usePolling(
    () => {
      void settleStaleReply();
    },
    15_000,
    {
      enabled: Boolean(threadId),
      deps: [settleStaleReply],
    },
  );

  useEffect(() => {
    const onVisible = () => {
      if (document.visibilityState === "visible")
        void settleStaleReply();
    };
    document.addEventListener("visibilitychange", onVisible);
    window.addEventListener("focus", onVisible);
    return () => {
      document.removeEventListener("visibilitychange", onVisible);
      window.removeEventListener("focus", onVisible);
    };
  }, [settleStaleReply]);

  // Interrupt the in-flight run for this thread. Best-effort: the backend stops
  // the agent and marks the run `cancelled`; the in-flight send then settles the
  // streaming bubble to the partial reply (see the cancelled branch in the send
  // pipeline), and refreshing `recentRun` clears `activeRunId` so the resume
  // effect reconciles. Safe to call when nothing is running (resolves to a no-op).
  const handleAbort = useCallback(async () => {
    if (!threadId)
      return;
    const runId
      = activeRunIdOf(recentRun)
        ?? messagesRef.current.find(
          message =>
            message.role === "assistant" && message.status === "streaming",
        )?.runId
        ?? null;
    if (!runId)
      return;
    try {
      await abortRun({ threadId, runId });
    }
    catch {
      // The run may already have finished; the refresh below still reconciles.
    }
    await refreshRecentRun(threadId);
    onThreadActivity();
  }, [threadId, recentRun, messagesRef, refreshRecentRun, onThreadActivity]);

  useEffect(() => {
    if (!thread || loadingThread || loadingStore || !pendingPrompt)
      return;
    if (consumedPromptRef.current === pendingPrompt.id || isPromptSendInFlight(pendingPrompt.id))
      return;
    // Only deliver the prompt to the thread it was composed for. A fast thread
    // switch during the (async) message load can make `thread` the newly-opened
    // conversation while this prompt still targets the one just created — sending
    // here would drop the first message (and its attachments) into the wrong
    // chat and persist it there. Wait for the target thread to be active.
    if (pendingPrompt.targetThreadId !== thread.id)
      return;

    const promptId = pendingPrompt.id;
    consumedPromptRef.current = promptId;
    markPromptSendInFlight(promptId);
    void handleSend({
      attachments: pendingPrompt.attachments ?? [],
      content: pendingPrompt.content,
    }, () => onPromptConsumed(promptId)).catch(() => {
      // A rejected first send is latched, not retried. Keeping it staged made
      // every later visit resend it with no visible trace of the attempt: the
      // Agent rejects before recording the message, so each visit added another
      // failed run, and any ref-anchored churn in this effect could repeat that
      // at run/status rate (see #1213). Consume it here and hand the text back
      // to the conversation's composer draft, so returning to the conversation
      // offers the message for an explicit resend — the same recovery a
      // composer-initiated rejection gets. The current view keeps its failed
      // bubble, whose retry action sends it again without leaving the thread.
      onPromptConsumed(promptId);
      restoreComposerDraft(pendingPrompt.targetThreadId, pendingPrompt);
    }).finally(() => clearPromptSendInFlight(promptId));
  }, [
    handleSend,
    loadingStore,
    loadingThread,
    onPromptConsumed,
    pendingPrompt,
    thread,
  ]);

  return {
    activeRunId,
    handleAbort,
    handleSend,
    handleQueuedSend,
    loadingThread,
    loadingIndicator,
    hasOlderHistory,
    historyTruncated,
    loadOlderHistory,
    loadAllHistoryForSearch,
    historyError,
    historyStatus,
    sessionChanged,
    retryHistory: () => threadId && reloadThreadHistory(threadId, true),
    messages,
    recentRun,
    renderWorkspace,
  };
}
