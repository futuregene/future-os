import type { Dispatch, MutableRefObject, SetStateAction } from "react";
import type { AgentMessage } from "./agentThreadTypes";
import { listen } from "@tauri-apps/api/event";
import { useEffect, useRef } from "react";
import { resetRunProjection, upsertStreamingPreview } from "./threadRunProjection";

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
  reloadMessagesQuiet: (targetThreadId: string, force?: boolean) => Promise<void>;
  // Generation counter shared with useThreadMessages: bump after every
  // streaming upsert so in-flight direct-replacement loads see the change
  // and discard themselves instead of clobbering the live bubble.
  messagesGenRef: MutableRefObject<number>;
}

/**
 * Re-attaches a live preview to a run this view didn't start: a conversation
 * backgrounded and returned to, one picked up after a reload, or one driven by a
 * remote (phone/web) client. Runtime deltas are pushed by Tauri in coalesced
 * batches; the initial snapshot handles events produced before subscription.
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
  messagesGenRef,
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
  // The `() => !cancelled` token handed to `upsertStreamingPreview` stops an
  // outgoing thread's in-flight snapshot from applying after a switch.
  useEffect(() => {
    if (!threadId || !activeRunId || sendingRef.current)
      return;

    const runId = activeRunId;
    const startedAt = activeRunStartedAt;
    let cancelled = false;
    const tick = () => {
      if (cancelled)
        return;
      void upsertStreamingPreview(runId, startedAt, setMessages, () => !cancelled).then(() => {
        // Bump the generation counter after every streaming upsert so
        // any in-flight direct-replacement load (loadFromAgent callers)
        // sees that state changed under them and discards its write
        // instead of clobbering the live bubble.
        messagesGenRef.current += 1;
      });
    };
    tick();
    const unlisten = listen<{
      threadId: string;
      runId: string;
      revision: number;
      status: string;
      resetProjection: boolean;
    }>("thread-runtime-updated", (event) => {
      if (cancelled || event.payload.threadId !== threadId || event.payload.runId !== runId)
        return;
      if (event.payload.resetProjection)
        resetRunProjection(runId);
      if (["completed", "failed", "cancelled"].includes(event.payload.status)) {
        void refreshRecentRun(threadId, workspaceId);
        void reloadMessagesQuiet(threadId, true);
        return;
      }
      tick();
    });
    // A terminal push can be lost (coalesce drop, backend restart). Without a
    // driver the live bubble would then freeze forever and the composer keep
    // showing "stop". This low-frequency pass re-reads the run row; if it has
    // actually settled, activeRunId flips to null and the settle effect above
    // force-reloads the persisted message. Harmless while still running.
    const selfHeal = setInterval(() => {
      if (cancelled || sendingRef.current)
        return;
      void refreshRecentRun(threadId, workspaceId);
    }, 30_000);
    // Close the registration race: a push that lands between the initial tick's
    // read and `listen` resolving would otherwise be missed until the next push.
    void unlisten.then(() => {
      if (!cancelled)
        tick();
    });

    return () => {
      cancelled = true;
      clearInterval(selfHeal);
      void unlisten.then(stop => stop());
    };
  }, [
    activeRunId,
    activeRunStartedAt,
    messagesGenRef,
    refreshRecentRun,
    reloadMessagesQuiet,
    sendingRef,
    setMessages,
    threadId,
    workspaceId,
  ]);

  // When a run this view was previewing (but did not itself start) settles,
  // reload the thread so the synthetic streaming bubble is replaced by the
  // persisted assistant message.
  useEffect(() => {
    const previous = prevActiveRunIdRef.current;
    prevActiveRunIdRef.current = activeRunId;
    if (previous && !activeRunId && !sendingRef.current && threadId) {
      // Force-reload: the streaming interval has already stopped.
      // Skipping the generation-counter guard ensures the persisted
      // assistant message replaces the synthetic bubble even if the
      // last tick bumped gen moments ago.
      void reloadMessagesQuiet(threadId, true);
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
