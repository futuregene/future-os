import type { StoredRun } from "../../integrations/storage/threadStore";
import type { AgentMessage } from "./agentThreadTypes";
import { useCallback, useEffect, useRef, useState } from "react";
import i18n from "../../i18n";
import { getLatestRun, getRun, getSessionEntries, listRuns } from "../../integrations/storage/threadStore";
import { invokeCommand } from "../../integrations/tauri/invoke";
import { errorMessage } from "../../lib/errors";
import { emitFutureEvent } from "../../lib/futureEvents";
import { upsertFutureReferenceData } from "../markdown/futureReferenceStore";
import { matchesSettledRun } from "./agentMessageFormatters";
import { entriesToMessages } from "./entryProjection";
import { applyRunMetadata, buildStreamingPreview, recoverAbortedTurns, recoverFailedRuns } from "./threadRunProjection";

interface UseThreadMessagesInput {
  threadId: string | null;
  workspaceId?: string | null;
  workspacePath?: string | null;
  agentSessionId?: string | null;
}

interface ThreadCacheEntry {
  messages: AgentMessage[];
  recentRun: StoredRun | null;
  /**
   * Raw entry count from the last getSessionEntries call; skips re-projection
   * on background refresh when unchanged.
   */
  entryCount: number;
}

type AgentLoadResult
  = | { status: "loaded"; messages: AgentMessage[]; entryCount: number }
    | { status: "empty"; entryCount: number }
    | { status: "failed"; error: string };

/** Max cached threads before evicting the oldest. */
const CACHE_MAX = 20;

/**
 * Drop the mid-run partial snapshot (the agent's save_callback persists each
 * completed LLM call while a run is in flight).  It sits after the last user
 * message, has no runId, and would render beside the live streaming bubble as a
 * duplicate.  The bubble re-projects the full event log, so nothing is lost.
 * Returns a new array when a snapshot was found, otherwise the input unchanged.
 */
function dropInFlightSnapshot(messages: AgentMessage[]): AgentMessage[] {
  const lastUserIdx = messages.map(m => m.role).lastIndexOf("user");
  if (lastUserIdx < 0)
    return messages;
  // Find the last assistant message after the last user message.
  for (let i = messages.length - 1; i > lastUserIdx; i--) {
    const message = messages[i]!;
    if (message.role === "assistant" && !message.runId && !isCompactionDivider(message)) {
      return messages.filter(m => m.id !== message.id);
    }
  }
  return messages;
}

/** A compaction divider is projected as an assistant message but is not a real turn. */
function isCompactionDivider(message: AgentMessage): boolean {
  return message.role === "assistant"
    && !message.content
    && message.segments?.length === 1
    && message.segments[0]?.kind === "compaction";
}

// Flash-free loading indicator (mirrors the right-context panel, useContextData):
// a thread load usually resolves in tens of ms, so hold off showing the "loading"
// text until the load has run this long...
const LOADING_INDICATOR_DELAY_MS = 200;
// ...and once shown, keep it visible at least this long so it can't itself flash.
const LOADING_INDICATOR_MIN_MS = 200;

/**
 * Owns a thread's message list + recent-run status: loads/restores messages on
 * thread switch, keeps a live run polling while one is active, and caches
 * recently-visited threads so switching back is instant.
 */
export function useThreadMessages({ threadId, workspaceId, workspacePath, agentSessionId }: UseThreadMessagesInput) {
  const [messages, setMessages] = useState<AgentMessage[]>([]);
  // The workspace the message tree renders against, committed in the same render
  // as `messages` (see the set points below). `thread.workspaceId` flips the
  // instant the user switches conversations but `messages` only update once the
  // load settles; if the message tree read the live workspace it would re-resolve
  // the stale messages' file links out of the new workspace for a torn frame (a
  // relative path flashing to its absolute form). Holding the workspace here — in
  // lockstep with `messages`, never derived from the live `thread` — means it can
  // only change in the same batch that swaps the messages, so no torn frame is
  // possible. Streaming / real-time writes are same-conversation and deliberately
  // leave this untouched.
  const [renderWorkspace, setRenderWorkspace] = useState(() => ({
    workspaceId: workspaceId ?? null,
    workspacePath: workspacePath ?? null,
  }));
  // Truthful data-loading state: gates pendingPrompt delivery (useAgentThreadState)
  // and must flip the instant a load starts/ends. The UI reads the debounced
  // `loadingIndicator` below instead, so this can stay honest without flashing.
  const [loadingThread, setLoadingThread] = useState(true);
  // Debounced projection of `loadingThread` for the "loading" indicator: only
  // turns on if a load outlasts the delay, and once on stays for a minimum so a
  // fast switch-back can't flash it. Purely presentational.
  const [loadingIndicator, setLoadingIndicator] = useState(false);
  const indicatorShownAtRef = useRef<number | null>(null);
  const [recentRun, setRecentRun] = useState<StoredRun | null>(null);

  // ── Generation counter for message writes ────────────────────────────
  // Functional-updater paths bump this after writing; direct-replacement
  // paths (loadFromAgent callers) snapshot it before the async work and
  // discard their write if it changed — so a streaming upsert or a real-time
  // user-message event that lands during an in-flight load isn't clobbered.
  const messagesGenRef = useRef<number>(0);

  // The thread's in-flight run, mirrored alongside `recentRun` so the load
  // path can fold the live streaming bubble into the history array it returns
  // (one setMessages — no history-then-bubble frame gap on switch). Updated
  // by refreshRecentRun; reset on every thread switch.
  const activeRunRef = useRef<{ runId: string | null; startedAt: number | null }>({
    runId: null,
    startedAt: null,
  });

  // In-memory cache of recently loaded threads. Switching back to a cached
  // thread restores messages instantly and then refreshes in the background.
  const cacheRef = useRef(new Map<string, ThreadCacheEntry>());
  // LRU order: most recently accessed threadId first.
  const lruRef = useRef<string[]>([]);

  function cachePut(tid: string, entry: ThreadCacheEntry) {
    const cache = cacheRef.current;
    if (!cache.has(tid) && cache.size >= CACHE_MAX) {
      const oldest = lruRef.current.pop();
      if (oldest)
        cache.delete(oldest);
    }
    cache.set(tid, entry);
    lruRef.current = [tid, ...lruRef.current.filter(id => id !== tid)];
  }

  function cacheGet(tid: string): ThreadCacheEntry | undefined {
    const entry = cacheRef.current.get(tid);
    if (entry) {
      lruRef.current = [tid, ...lruRef.current.filter(id => id !== tid)];
    }
    return entry;
  }

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
      // One row, not the thread's whole run history. Invoked on push events
      // (thread-runtime-updated terminal / remote-activity) and loads — there is
      // no longer a periodic timer driving it.
      const latestRun = await getLatestRun(targetThreadId);
      if (generation !== recentRunGenRef.current) {
        return;
      }
      // Mirror the in-flight run for the load path (see activeRunRef).
      if (latestRun && targetThreadId === activeThreadIdRef.current) {
        activeRunRef.current = {
          runId: matchesSettledRun(latestRun.status) ? null : latestRun.id,
          startedAt: latestRun.startedAt ?? latestRun.createdAt ?? null,
        };
      }
      if (targetThreadId === activeThreadIdRef.current) {
        setRecentRun(latestRun);
      }
      if (latestRun) {
        upsertFutureReferenceData(targetWorkspaceId, "run", latestRun.id, latestRun);
      }
    }
    catch {
      // Run-status refresh is best-effort.
    }
  }, []);

  // Reload a thread's messages from the agent session (the sole source of truth)
  // without flipping the full-screen loading state — used to swap a synthetic
  // streaming bubble for the persisted assistant message once a background run
  // settles. Keeps the current messages if the agent has nothing (never blanks).
  //
  // `force` (default false) skips the generation-counter guard.  Callers that
  // are the authoritative settle writer (useRunReattach settle effect) pass
  // `true` because at that point the streaming interval has already stopped and
  // no further ticks will repair a discarded write.
  const reloadMessagesQuiet = useCallback(async (targetThreadId: string, force = false) => {
    const gen = force ? undefined : messagesGenRef.current;
    // Thread switch to an active conversation needs the live bubble folded in
    // now — read it from the state we already hold.
    const result = await loadFromAgent(targetThreadId, undefined, activeRunRef.current.runId, activeRunRef.current.startedAt);
    if (result.status !== "loaded" || targetThreadId !== activeThreadIdRef.current)
      return;
    // If another writer bumped the generation counter while we were in-flight
    // (streaming upsert, real-time user message), our snapshot-based array would
    // overwrite that update — discard it instead; the live path is authoritative.
    if (!force && gen !== undefined && messagesGenRef.current !== gen)
      return;
    setMessages(result.messages);
    cachePut(targetThreadId, { messages: result.messages, recentRun: null, entryCount: result.entryCount });
    // loadFromAgent is a hoisted inner function; this reload fires only on
    // explicit call, so it's intentionally excluded from the deps.
    // eslint-disable-next-line react/exhaustive-deps
  }, []);

  // Reconstruct a thread's messages from the agent session JSONL
  // (get_session_entries) — the only message store (the SQLite messages table
  // was removed). Empty and failed loads stay distinct so a transient Agent
  // error never masquerades as an empty conversation and clears valid cache.
  async function loadFromAgent(
    tid: string,
    wid?: string | null,
    activeRunId?: string | null,
    activeRunStartedAt?: number | null,
  ): Promise<AgentLoadResult> {
    try {
      const result = await getSessionEntries(tid);
      const entryCount = result?.entries?.length ?? 0;
      if (!entryCount)
        return { status: "empty", entryCount: 0 };
      const messages = entriesToMessages(result.entries as unknown as import("./entryProjection").SessionEntry[]);
      if (!messages.length)
        return { status: "empty", entryCount };
      // Agent JSONL doesn't record a run's GUI-side outcome (failed/cancelled/
      // model) — backfill it from the SQLite `runs` table so a reload keeps the
      // Retry/Continue button, the "stopped" marker, and the model badge.
      const runs = await listRuns(tid).catch(() => [] as StoredRun[]);
      const withRunMeta = applyRunMetadata(messages, runs);
      // An aborted turn has no reply in the session JSONL — recover the partial
      // text the model streamed (persisted as run events) so it isn't lost.
      const recovered = await recoverAbortedTurns(withRunMeta);
      // A run that failed before any assistant entry was saved (e.g. the model
      // API rejected the first call) leaves no trace in the session JSONL —
      // rebuild its failure bubble from the run record so the error survives a
      // thread switch instead of silently disappearing.
      const withFailures = recoverFailedRuns(recovered, runs);
      // An in-flight run for this thread is folded into the SAME array here:
      // history and live bubble land in one setMessages, so switching to an
      // active conversation paints both in a single frame instead of history
      // then bubble. Verified against the run row so a settle that raced the
      // reload never resurrects a bubble for a finished run.
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
        ? [...withFailures, liveBubble]
        : withFailures;
      await refreshRecentRun(tid, wid).catch(() => {});
      return { status: "loaded", messages: finalMessages, entryCount };
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
        setRenderWorkspace({ workspaceId: null, workspacePath: null });
        setLoadingThread(false);
        return;
      }
      // The thread is changing: never carry the previous thread's in-flight
      // run into this one's load (the pre-warm below only refreshes when the
      // mirror is empty, so a stale run must be cleared here or it would be
      // folded into the wrong conversation's history).
      activeRunRef.current = { runId: null, startedAt: null };

      // Check cache first — restore instantly if available, then refresh.
      const cached = cacheGet(threadId);
      if (cached) {
        setMessages(cached.messages);
        setRecentRun(cached.recentRun);
        setRenderWorkspace({ workspaceId: workspaceId ?? null, workspacePath: workspacePath ?? null });
        setLoadingThread(false);
        // Background refresh from the agent session (empty when it has none).
        // Use a functional updater that preserves any live streaming bubble
        // (stream_<runId>) that was inserted by useRunReattach while the async
        // load was in flight — a plain setMessages(restored) would clobber it.
        // Pre-warm the in-flight-run mirror so the load below folds the live
        // streaming bubble into the returned history (one render — no
        // history-then-bubble frame gap when the active thread is running).
        if (!activeRunRef.current.runId) {
          await refreshRecentRun(threadId, workspaceId).catch(() => {});
        }
        const result = await loadFromAgent(
          threadId,
          workspaceId,
          activeRunRef.current.runId,
          activeRunRef.current.startedAt,
        );
        if (!cancelled && threadId === activeThreadIdRef.current && result.status !== "failed") {
          // Entry count unchanged and no in-flight run — nothing new since we
          // cached; skip the projection+merge entirely (the old fast path).
          // With an in-flight run we MUST fall through: the fresh projection
          // below folds the live streaming bubble, landing history + bubble in
          // one render instead of the reattach tick's history-then-bubble.
          if (result.entryCount === cached.entryCount && !activeRunRef.current.runId)
            return;
          const restored = result.status === "loaded" ? result.messages : [];
          setMessages((current) => {
            // Preserve streaming bubbles whose run hasn't settled yet — but
            // never one already folded into `restored` by the load (dedup).
            const settledRunIds = new Set(restored.filter(m => m.runId).map(m => m.runId));
            const keepBubbles = current.filter(
              m => m.id.startsWith("stream_")
                && !settledRunIds.has(m.runId)
                && !restored.some(r => r.id === m.id),
            );
            // When a streaming bubble is alive, the agent's save_callback may
            // have persisted a mid-run partial snapshot of the same turn (an
            // assistant message with no runId at the tail).  Drop it so the
            // turn renders once — the live bubble re-projects the full event
            // log, so nothing is lost.
            let restoredOut = restored;
            if (keepBubbles.length > 0) {
              restoredOut = dropInFlightSnapshot(restored);
            }
            // Streaming bubbles go at the end, after all persisted history.
            return [...restoredOut, ...keepBubbles];
          });
          cachePut(threadId, { messages: restored, recentRun: null, entryCount: result.entryCount });
        }
        return;
      }

      setLoadingThread(true);
      // Snapshot gen so a real-time user message that arrives during the
      // first load doesn't get overwritten by the freshly-projected array.
      const firstGen = messagesGenRef.current;
      // Pre-warm the in-flight-run mirror (see the cache-hit branch).
      if (!activeRunRef.current.runId) {
        await refreshRecentRun(threadId, workspaceId).catch(() => {});
      }
      const result = await loadFromAgent(
        threadId,
        workspaceId,
        activeRunRef.current.runId,
        activeRunRef.current.startedAt,
      );
      if (!cancelled) {
        if (messagesGenRef.current !== firstGen) {
          // A concurrent writer (real-time user message) bumped gen while we
          // were loading — our snapshot is stale and the concurrent message
          // state is authoritative for the target thread. Commit its workspace
          // in the same render before leaving the old message tree behind.
          setRenderWorkspace({ workspaceId: workspaceId ?? null, workspacePath: workspacePath ?? null });
          setLoadingThread(false);
          return;
        }
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
          const restoredMessages = result.status === "loaded" ? result.messages : [];
          setMessages(restoredMessages);
          cachePut(threadId, { messages: restoredMessages, recentRun: null, entryCount: result.entryCount });
        }
        setRenderWorkspace({ workspaceId: workspaceId ?? null, workspacePath: workspacePath ?? null });
        setLoadingThread(false);
      }
    }

    void loadThreadMessages();

    return () => {
      cancelled = true;
    };
    // loadFromAgent is an unstable inner function; the reload must fire on
    // thread/workspace change only, not on every render, so it's excluded.
    // eslint-disable-next-line react/exhaustive-deps
  }, [refreshRecentRun, workspaceId, workspacePath, threadId]);

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
    if (!threadId || !agentSessionId)
      return;
    const handler = (ev: Event) => {
      const detail = (ev as CustomEvent).detail as {
        sessionId: string;
        eventType: string;
        payload: Record<string, unknown>;
      } | undefined;
      if (!detail || detail.sessionId !== agentSessionId)
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
      if (detail.eventType !== "user_message")
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
      // Bump the generation counter so any in-flight direct replacement
      // (loadFromAgent callers) sees that state moved under them and discards.
      messagesGenRef.current += 1;
    };
    window.addEventListener("future:agent-event", handler);
    return () => window.removeEventListener("future:agent-event", handler);
  }, [
    agentSessionId,
    isRunActive,
    refreshRecentRun,
    reloadMessagesQuiet,
    threadId,
    workspaceId,
  ]);

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
