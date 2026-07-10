import type { Dispatch, MutableRefObject, SetStateAction } from "react";
import type { AgentMessage } from "./agentThreadTypes";
import { listen } from "@tauri-apps/api/event";
import { useEffect, useRef } from "react";
import { upsertStreamingPreview } from "./threadRunProjection";

interface UseRunReattachInput {
  threadId: string | null;
  workspaceId?: string | null;
  // The run this thread is executing but this view did not itself start (or is
  // no longer driving); null once it settles.
  activeRunId: string | null;
  // Epoch-ms anchor for the re-attached run's live elapsed timer.
  activeRunStartedAt: number | null;
  // In-flight lock owned by the parent: while a local send owns the view it
  // renders the stream itself, so every re-attach path skips.
  sendingRef: MutableRefObject<boolean>;
  setMessages: Dispatch<SetStateAction<AgentMessage[]>>;
  refreshRecentRun: (threadId: string, workspaceId?: string | null) => Promise<void>;
  reloadMessagesQuiet: (targetThreadId: string) => Promise<void>;
}

/**
 * Re-attaches a live preview to a run this view didn't start: a conversation
 * backgrounded and returned to, one picked up after a reload, or one driven by a
 * remote (phone/web) client. Polls the streaming bubble in, reloads the thread
 * when the run settles, and listens for the backend's remote-activity signal.
 */
export function useRunReattach({
  threadId,
  workspaceId,
  activeRunId,
  activeRunStartedAt,
  sendingRef,
  setMessages,
  refreshRecentRun,
  reloadMessagesQuiet,
}: UseRunReattachInput) {
  const prevActiveRunIdRef = useRef<string | null>(null);

  useEffect(() => {
    return () => {
      // Drop the run tracked for the outgoing thread so the incoming thread's
      // settle detection doesn't fire against a stale run id.
      prevActiveRunIdRef.current = null;
    };
  }, [threadId]);

  // Re-attach a live preview to a run that this view didn't start (or is no
  // longer driving): a conversation started, backgrounded, and returned to while
  // still running, or one picked up after an app reload. While a local send owns
  // the view (`sendingRef`), that path renders the stream itself, so skip.
  //
  // The poll UPSERTS the streaming bubble every tick (not a one-time insert) so it
  // survives a `loadThreadMessages` array-replace that lands mid-stream — the next
  // tick simply re-inserts it. That resilience is what makes a returned-to run
  // reconnect instead of showing an empty bubble the reload silently dropped.
  //
  // Deliberately hand-rolled rather than `usePolling`: the `() => !cancelled`
  // token handed to `upsertStreamingPreview` is the only guard that stops a
  // stale run's in-flight async upsert from applying after a thread/run switch —
  // exactly the race-sensitive async case `usePolling` documents it can't cover.
  useEffect(() => {
    if (!threadId || !activeRunId || sendingRef.current)
      return;

    const runId = activeRunId;
    const startedAt = activeRunStartedAt;
    let cancelled = false;
    const tick = () => {
      if (cancelled)
        return;
      void upsertStreamingPreview(runId, startedAt, setMessages, () => !cancelled);
    };
    tick();
    const timer = window.setInterval(tick, 220);

    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
  }, [activeRunId, activeRunStartedAt, sendingRef, setMessages, threadId]);

  // When a run this view was previewing (but did not itself start) settles,
  // reload the thread so the synthetic streaming bubble is replaced by the
  // persisted assistant message.
  useEffect(() => {
    const previous = prevActiveRunIdRef.current;
    prevActiveRunIdRef.current = activeRunId;
    if (previous && !activeRunId && !sendingRef.current && threadId) {
      void reloadMessagesQuiet(threadId);
    }
  }, [activeRunId, reloadMessagesQuiet, sendingRef, threadId]);

  // A remote (phone/web) client can drive this thread's session in the
  // background. This view never started that run, and the recent-run poll
  // only self-sustains once a run is already in flight — so a fresh remote run
  // on an idle foreground thread would otherwise go unnoticed here (only the
  // sidebar's independent run-status poll would spin). On the backend's
  // remote-activity signal for THIS thread, pull the new run (which arms the
  // live-preview + settle-reload machinery) and reload messages so the phone's
  // user bubble shows immediately. Skip while a local send owns the view.
  useEffect(() => {
    if (!threadId)
      return;
    let cancelled = false;
    const unlisten = listen<string>("remote-activity", (event) => {
      if (cancelled || event.payload !== threadId || sendingRef.current)
        return;
      void refreshRecentRun(threadId, workspaceId);
      void reloadMessagesQuiet(threadId);
    });
    return () => {
      cancelled = true;
      void unlisten.then(stop => stop());
    };
  }, [refreshRecentRun, reloadMessagesQuiet, sendingRef, workspaceId, threadId]);
}
