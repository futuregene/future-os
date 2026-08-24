import type { AgentMessage } from "@future-os/thread-projection";
import type { StoredRun } from "../../integrations/storage/threadStore";
import { entriesToMessages, matchesSettledRun } from "@future-os/thread-projection";
import { useCallback, useEffect, useRef, useState } from "react";
import i18n from "../../i18n";
import { getLatestRun, getRun, getSessionEntries, listRuns } from "../../integrations/storage/threadStore";
import { invokeCommand } from "../../integrations/tauri/invoke";
import { errorMessage } from "../../lib/errors";
import { emitFutureEvent } from "../../lib/futureEvents";
import { applyRunMetadata, buildStreamingPreview, mergeStreamingPreview, recoverAbortedTurns, recoverFailedRuns } from "./threadRunProjection";

interface UseThreadMessagesInput {
  threadId: string | null;
  workspaceId?: string | null;
  workspacePath?: string | null;
  agentSessionId?: string | null;
}

type AgentLoadResult
  = | { status: "loaded"; messages: AgentMessage[] }
    | { status: "empty" }
    | { status: "failed"; error: string };

// Flash-free loading indicator (mirrors the right-context panel, useContextData):
// a thread load usually resolves in tens of ms, so hold off showing the "loading"
// text until the load has run this long...
const LOADING_INDICATOR_DELAY_MS = 200;
// ...and once shown, keep it visible at least this long so it can't itself flash.
const LOADING_INDICATOR_MIN_MS = 200;

/**
 * Owns a thread's message list + recent-run status: loads messages when the
 * instance mounts and keeps a live run ticking while one is active.
 *
 * AgentThread is keyed by thread id, so each conversation gets its own
 * instance: every state and listener here belongs to exactly one thread, and
 * writers from a conversation the user switched away from are torn down with
 * their instance — no cross-thread guarding is needed. The races left are
 * within this one thread (see `messagesGenRef`).
 */
export function useThreadMessages({ threadId, workspaceId, workspacePath, agentSessionId }: UseThreadMessagesInput) {
  const normalizedAgentSessionId = agentSessionId?.trim() || null;
  const [messages, setMessages] = useState<AgentMessage[]>([]);
  // Truthful data-loading state: gates pendingPrompt delivery (useAgentThreadState)
  // and must flip the instant a load starts/ends. The UI reads the debounced
  // `loadingIndicator` below instead, so this can stay honest without flashing.
  const [loadingThread, setLoadingThread] = useState(true);
  const loadingRef = useRef(loadingThread);
  loadingRef.current = loadingThread;
  // Debounced projection of `loadingThread` for the "loading" indicator: only
  // comes on if a load outlasts the delay, and once on stays for a minimum so a
  // fast switch-back can't flash it. Purely presentational.
  const [loadingIndicator, setLoadingIndicator] = useState(false);
  const indicatorShownAtRef = useRef<number | null>(null);
  const [recentRun, setRecentRun] = useState<StoredRun | null>(null);

  // ── Generation counter for message writes ────────────────────────────
  // Within this one thread, a quiet reload (direct replacement) can be in
  // flight while a real-time user message appends: the reload snapshots this
  // counter before its async work and discards its write if the counter
  // moved, so the live append wins. Append writers bump it after writing.
  const messagesGenRef = useRef<number>(0);

  // The thread's in-flight run, mirrored alongside `recentRun` so the load
  // path can fold the live streaming bubble into the history array it returns
  // (one setMessages — no history-then-bubble frame gap).
  const activeRunRef = useRef<{ runId: string | null; startedAt: number | null }>({
    runId: null,
    startedAt: null,
  });

  // Guard against overlapping refreshes (poll tick, send, reattach) where a
  // slow response lands after a newer one and writes stale run state. Newest
  // call wins.
  const recentRunGenRef = useRef(0);
  const refreshRecentRun = useCallback(async (targetThreadId: string, _targetWorkspaceId?: string | null) => {
    const generation = ++recentRunGenRef.current;
    try {
      // One row, not the thread's whole run history. Invoked on push events
      // (thread-runtime-updated terminal / remote-activity) and loads — there is
      // no longer a periodic timer driving it.
      const latestRun = await getLatestRun(targetThreadId);
      if (generation !== recentRunGenRef.current) {
        return;
      }
      // Mirror the in-flight run for the load path (see activeRunRef).
      activeRunRef.current = {
        runId: latestRun && !matchesSettledRun(latestRun.status) ? latestRun.id : null,
        startedAt: latestRun?.startedAt ?? latestRun?.createdAt ?? null,
      };
      setRecentRun(latestRun);
    }
    catch {
      // Run-status refresh is best-effort.
    }
  }, []);

  // Reload the thread's messages from the agent session (the sole source of
  // truth) without flipping the full-screen loading state — used to swap a
  // synthetic streaming bubble for the persisted assistant message once a
  // background run settles. Keeps the current messages if the agent has
  // nothing (never blanks).
  //
  // `force` (default false) skips the generation-counter guard.  Callers that
  // are the authoritative settle writer (useRunReattach settle effect) pass
  // `true` because at that point the streaming interval has already stopped and
  // no further ticks will repair a discarded write.
  const reloadMessagesQuiet = useCallback(async (targetThreadId: string, force = false) => {
    const gen = force ? undefined : messagesGenRef.current;
    const result = await loadFromAgent(targetThreadId, undefined, activeRunRef.current.runId, activeRunRef.current.startedAt);
    if (result.status !== "loaded")
      return;
    // If a real-time user message bumped the counter while we were in-flight,
    // our snapshot-based array would overwrite that append — discard it
    // instead; the live path is authoritative.
    if (gen !== undefined && messagesGenRef.current !== gen)
      return;
    setMessages(result.messages);
    // loadFromAgent is a hoisted inner function; this reload fires only on
    // explicit call, so it's intentionally excluded from the deps.
    // eslint-disable-next-line react/exhaustive-deps
  }, []);

  // Reconstruct the thread's messages from the agent session JSONL
  // (get_session_entries) — the only message store (the SQLite messages table
  // was removed). Empty and failed loads stay distinct so a transient Agent
  // error never masquerades as an empty conversation.
  async function loadFromAgent(
    tid: string,
    wid?: string | null,
    activeRunId?: string | null,
    activeRunStartedAt?: number | null,
  ): Promise<AgentLoadResult> {
    try {
      const result = await getSessionEntries(tid);
      const entries = result?.entries ?? [];
      if (!entries.length)
        return { status: "empty" };
      const messages = entriesToMessages(entries as unknown as import("@future-os/thread-projection").SessionEntry[]);
      if (!messages.length)
        return { status: "empty" };
      // Agent JSONL doesn't record a run's GUI-side outcome (failed/cancelled/
      // model) — backfill it from the SQLite `runs` table so a reload keeps the
      // Retry/Continue button, the "stopped" marker, and the model badge.
      const runs = await listRuns(tid).catch(() => [] as StoredRun[]);
      const withRunMeta = applyRunMetadata(messages, runs);
      // An aborted exchange has no reply in the session JSONL — recover the partial
      // text the model streamed (persisted as run events) so it isn't lost.
      const recovered = await recoverAbortedTurns(withRunMeta);
      // A run that failed before any assistant entry was saved (e.g. the model
      // API rejected the first call) leaves no trace in the session JSONL —
      // rebuild its failure bubble from the run record so the error survives a
      // thread switch instead of silently disappearing.
      const withFailures = recoverFailedRuns(recovered, runs);
      // An in-flight run is folded into the SAME array here: history and live
      // bubble land in one setMessages, so opening an active conversation
      // paints both in a single frame instead of history then bubble. The fold
      // also dedups a mid-run snapshot the agent's save_callback may already
      // have persisted for this exchange (mergeStreamingPreview). Verified
      // against the run row so a settle that raced the reload never resurrects
      // a bubble for a finished run.
      const liveBubble = activeRunId
        ? await getRun(activeRunId)
            .then(run =>
              run && !matchesSettledRun(run.status)
                ? buildStreamingPreview(activeRunId, activeRunStartedAt ?? null)
                : null,
            )
            .catch(() => null)
        : null;
      const finalMessages = liveBubble
        ? mergeStreamingPreview(withFailures, liveBubble)
        : withFailures;
      await refreshRecentRun(tid, wid).catch(() => {});
      return { status: "loaded", messages: finalMessages };
    }
    catch (error) {
      return { status: "failed", error: errorMessage(error) };
    }
  }

  useEffect(() => {
    let cancelled = false;

    async function loadThreadMessages() {
      if (!threadId) {
        setMessages([]);
        setLoadingThread(false);
        return;
      }
      setLoadingThread(true);
      // Pre-warm the in-flight-run mirror so loadFromAgent folds the live
      // streaming bubble into the returned history (one render — no
      // history-then-bubble frame gap when the thread is running).
      await refreshRecentRun(threadId, workspaceId).catch(() => {});
      const result = await loadFromAgent(threadId, workspaceId, activeRunRef.current.runId, activeRunRef.current.startedAt);
      if (cancelled)
        return;
      if (result.status === "failed") {
        setMessages([
          {
            id: "store_error",
            role: "assistant",
            authorKey: "author.system",
            content: i18n.t("agent:thread.messagesLoadFailed", { message: result.error }),
            createdAt: new Date().toISOString(),
          },
        ]);
      }
      else {
        setMessages(result.status === "loaded" ? result.messages : []);
      }
      setLoadingThread(false);
    }

    void loadThreadMessages();

    return () => {
      cancelled = true;
    };
    // loadFromAgent is an unstable inner function; the load must fire on
    // thread/workspace change only, not on every render, so it's excluded.
    // eslint-disable-next-line react/exhaustive-deps
  }, [refreshRecentRun, workspaceId, threadId]);

  // Derive the flash-free indicator from the truthful `loadingThread`: show it
  // only if loading outlasts LOADING_INDICATOR_DELAY_MS, and once shown hold it
  // for at least LOADING_INDICATOR_MIN_MS so it can't flash off immediately.
  useEffect(() => {
    if (loadingThread) {
      const showTimer = setTimeout(() => {
        indicatorShownAtRef.current = performance.now();
        setLoadingIndicator(true);
      }, LOADING_INDICATOR_DELAY_MS);
      return () => clearTimeout(showTimer);
    }
    // Loading finished. If the indicator never appeared, just keep it hidden.
    if (indicatorShownAtRef.current === null) {
      setLoadingIndicator(false);
      return;
    }
    // It's showing — hold it for the remainder of its minimum visible duration.
    const remaining = LOADING_INDICATOR_MIN_MS - (performance.now() - indicatorShownAtRef.current);
    if (remaining <= 0) {
      indicatorShownAtRef.current = null;
      setLoadingIndicator(false);
      return;
    }
    const hideTimer = setTimeout(() => {
      indicatorShownAtRef.current = null;
      setLoadingIndicator(false);
    }, remaining);
    return () => clearTimeout(hideTimer);
  }, [loadingThread]);

  const isRunActive = Boolean(recentRun && !matchesSettledRun(recentRun.status));

  // Remote runs are discovered from the already-open session event stream.
  // This replaces the old per-thread 2s get_state poll.
  const attachedRef = useRef(false);

  // ── Real-time user_message from StreamEvents observer ────────────
  // Inserts the user message directly from the Tauri event stream
  // for zero-latency display.  All other events (text_chunk, thinking,
  // tools, agent_end) continue through the synthetic run → useRunReattach
  // path to avoid conflicting with the existing streaming bubble logic.
  useEffect(() => {
    if (!threadId || !normalizedAgentSessionId)
      return;
    const handler = (ev: Event) => {
      const detail = (ev as CustomEvent).detail as {
        threadId: string;
        sessionId: string;
        eventType: string;
        payload: Record<string, unknown>;
      } | undefined;
      // Only this conversation's events apply to this instance — other
      // conversations live on their own keyed AgentThread instances.
      if (!detail || detail.threadId !== threadId || detail.sessionId !== normalizedAgentSessionId)
        return;
      if (detail.eventType === "agent_end") {
        attachedRef.current = false;
        emitFutureEvent("agent_end", undefined);
        return;
      }
      if (detail.eventType === "agent_start") {
        if (isRunActive || attachedRef.current)
          return;
        attachedRef.current = true;
        void invokeCommand<{ runId?: string }>("attach_remote_stream", { threadId })
          .then(async (result) => {
            if (!result?.runId)
              return;
            await reloadMessagesQuiet(threadId);
            await refreshRecentRun(threadId, workspaceId);
          })
          .catch(() => {
            attachedRef.current = false;
          });
        return;
      }
      if (
        detail.eventType === "compaction_started"
        || detail.eventType === "compaction_committed"
        || detail.eventType === "compaction_failed"
      ) {
        // Run-scoped compaction is already projected from the persisted run
        // event log. This direct path is for standalone/manual and model-switch
        // compaction, which has no active run bubble to host its status.
        if (isRunActive || attachedRef.current)
          return;
        const operationId = typeof detail.payload.operation_id === "string"
          ? detail.payload.operation_id
          : `session_${Date.now()}`;
        const messageId = `compaction_${operationId}`;
        const checkpointId = typeof detail.payload.checkpoint_id === "string"
          ? detail.payload.checkpoint_id
          : operationId;
        const tokensBefore = typeof detail.payload.tokens_before === "number"
          ? detail.payload.tokens_before
          : undefined;
        const error = typeof detail.payload.error === "string"
          ? detail.payload.error
          : undefined;
        const trigger = typeof detail.payload.trigger === "string"
          ? detail.payload.trigger
          : undefined;
        const status = detail.eventType === "compaction_started"
          ? "running" as const
          : detail.eventType === "compaction_failed"
            ? "failed" as const
            : "completed" as const;
        setMessages((prev) => {
          const segment = {
            id: checkpointId,
            kind: "compaction" as const,
            ...(tokensBefore != null && tokensBefore > 0 ? { tokensBefore } : {}),
            ...(trigger ? { trigger } : {}),
            ...(status !== "completed" ? { status } : {}),
            ...(error ? { error } : {}),
          };
          const existing = prev.findIndex(message => message.id === messageId);
          const message: AgentMessage = {
            id: messageId,
            role: "assistant",
            authorKey: "author.researchCopilot",
            content: "",
            status: "complete",
            createdAt: new Date().toISOString(),
            segments: [segment],
          };
          if (existing < 0)
            return [...prev, message];
          const next = [...prev];
          next[existing] = message;
          return next;
        });
        messagesGenRef.current += 1;
        return;
      }
      if (detail.eventType !== "user_message")
        return;

      // A user_message that lands while this thread's load is still in flight
      // would append onto the not-yet-committed base — dropping it is
      // lossless, the projected JSONL load carries the entry.
      if (loadingRef.current)
        return;

      const text = typeof detail.payload.text === "string" ? detail.payload.text : "";
      if (!text)
        return;
      setMessages((prev) => {
        // Dedup: skip if the last user message has identical text.
        // Checking only the last message avoids suppressing legitimate
        // repeated prompts (e.g. sending "continue" twice).
        const userMsgs = prev.filter(m => m.role === "user");
        const lastUser = userMsgs[userMsgs.length - 1];
        if (lastUser && lastUser.content === text)
          return prev;
        return [...prev, {
          id: `user_${Date.now()}`,
          role: "user",
          authorKey: "author.you",
          content: text,
          status: "complete",
          createdAt: new Date().toISOString(),
        } satisfies AgentMessage];
      });
      // Bump the generation counter so an in-flight quiet reload sees that
      // state moved under it and discards its replacement instead of
      // clobbering this append.
      messagesGenRef.current += 1;
    };
    window.addEventListener("future:agent-event", handler);
    return () => window.removeEventListener("future:agent-event", handler);
  }, [
    normalizedAgentSessionId,
    isRunActive,
    refreshRecentRun,
    reloadMessagesQuiet,
    threadId,
    workspaceId,
  ]);

  // The message tree renders against this thread's own workspace, so file
  // links always resolve against the right root — with the view keyed by
  // thread, messages and workspace can no longer disagree.
  const renderWorkspace = {
    workspaceId: workspaceId ?? null,
    workspacePath: workspacePath ?? null,
  };

  return {
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
  };
}
