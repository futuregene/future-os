import type { AgentMessage } from "@future-os/thread-projection";
import type { Dispatch, MutableRefObject, SetStateAction } from "react";
import type { StoredRun, StoredThread } from "../../integrations/storage/threadStore";
import type { ComposerSendPayload } from "./Composer";
import { useCallback, useEffect, useRef } from "react";
import i18n from "../../i18n";
import { errorMessage } from "../../lib/errors";
import { emitFutureEvent } from "../../lib/futureEvents";
import { runSendPipeline } from "./sendPipeline";

/**
 * The policy a queued send asks for: the Agent holds the prompt and folds a
 * burst of such sends into ONE run at the turn boundary — the `enqueue_if_busy`
 * follow-up semantics a chat composer wants, without a second turn per message.
 */
export const QUEUED_SEND_POLICY = "enqueue_coalescing";

interface UseSendMessageInput {
  thread: StoredThread | null;
  modelId: string;
  thinkingLevel: string;
  // A run already in flight this view is only re-attached to (backgrounded,
  // reloaded, or remote-driven); a fresh send while it runs must be rejected.
  activeRunId: string | null;
  // In-flight lock shared with the re-attach hook (which skips while a local
  // send owns the view). Owned by the parent so both units see the same flag.
  sendingRef: MutableRefObject<boolean>;
  setMessages: Dispatch<SetStateAction<AgentMessage[]>>;
  setRecentRun: (run: StoredRun) => void;
  refreshRecentRun: (threadId: string) => Promise<void>;
  onThreadActivity: () => void;
}

/**
 * The queued-send entry point: a message the reader wrote while the reply was
 * still streaming.
 *
 * It leaves NOW and the Agent holds it — `enqueue_coalescing` makes a burst of
 * such messages fold into ONE run at the turn boundary, which is what the old
 * client-side queue used to do by joining the texts itself. Keeping it on the
 * Agent is what makes the message survive this window: a reload, a renderer
 * crash, or a second client on the same conversation all still see the row in
 * `get_state`'s `queuedRuns`, and the run happens whether or not this view is
 * still around to flush it.
 *
 * The queued ack comes back as `run_queued`, which the pipeline already reads as
 * "accepted and alive" — so the optimistic bubble stays, waiting for its turn,
 * instead of being settled as a cut-off reply.
 */
export function useQueuedSend({
  thread,
  modelId,
  thinkingLevel,
  setMessages,
  refreshRecentRun,
  onThreadActivity,
}: Omit<UseSendMessageInput, "activeRunId" | "sendingRef" | "setRecentRun">) {
  return useCallback(async (payload: ComposerSendPayload) => {
    if (!thread)
      return false;
    try {
      // Deliberately outside the one-send-at-a-time lock: the run it queues
      // behind is exactly the one holding `sendingRef`, and bumping the send
      // generation would cut that run's pipeline off from this view. Every view
      // write below is guarded by the pipeline's own optimistic ids, so the two
      // sends cannot overwrite each other's rows.
      await runSendPipeline(
        {
          busyPolicy: QUEUED_SEND_POLICY,
          isCurrentSend: () => true,
          modelId,
          onThreadActivity,
          refreshRecentRun,
          setMessages,
          thinkingLevel,
          thread,
          // NOT into `recentRun`: that is "the run this view is executing", and
          // the queued one has not started. Letting it in would point the
          // re-attach path at a run with no events instead of the one still
          // streaming. The rail learns about it from the refresh below.
          setRecentRun: () => {},
        },
        payload,
      );
      // The row exists now (`queued`), so the conversation list and this view's
      // status line should say so before the run even starts.
      await refreshRecentRun(thread.id).catch(() => {});
    }
    catch (error) {
      // The pipeline has already put the failure in the thread (a failed bubble,
      // as a plain send would); this is the extra sentence for the case where it
      // refused before that, so a queued message never disappears silently.
      emitFutureEvent("toast", { message: errorMessage(error), tone: "error" });
    }
    return true;
  }, [modelId, onThreadActivity, refreshRecentRun, setMessages, thinkingLevel, thread]);
}

/** The identity of one send, held from before `createRun` answers. */
interface LocalSend {
  /** The run this send created, once `createRun` has answered. */
  runId: string | null;
  /**
   * Settles the promise this send returned to its caller. Called only by
   * {@link useSendMessage}'s `abandonSend` — that caller cannot rely on the
   * pipeline settling, which is the whole point of holding it here.
   */
  releaseDelivery: () => void;
}

/**
 * The send entry point. Guards one prompt at a time per session, stamps a send
 * generation so a superseded send stops mutating the view, then delegates the
 * heavy lifting to {@link runSendPipeline}. Releasing the lock only when this
 * send still owns it keeps a newer send/thread switch in control of the flag.
 */
export function useSendMessage({
  thread,
  modelId,
  thinkingLevel,
  activeRunId,
  sendingRef,
  setMessages,
  setRecentRun,
  refreshRecentRun,
  onThreadActivity,
}: UseSendMessageInput) {
  const sendGenerationRef = useRef(0);
  // Object identity identifies this send even before createRun completes.
  const localSendRef = useRef<LocalSend | null>(null);

  const handleSend = useCallback(async (payload: ComposerSendPayload, onAccepted?: () => void) => {
    if (!thread) {
      if (onAccepted)
        throw new Error("No active conversation");
      return;
    }
    // One prompt at a time per session. `sendingRef` guards a send this
    // view started; `activeRunId` guards a run already in flight that this view
    // is only re-attached to (backgrounded, reloaded, or remote-driven). Every
    // send path — composer, `recover-run`, pendingPrompt — funnels through here,
    // so a second prompt while one runs would be rejected by the agent
    // ("already running") and leave a stray failed run. Reject it up front, with
    // visible feedback instead of a silent drop.
    if (sendingRef.current || activeRunId) {
      const message = i18n.t("agent:thread.alreadyRunning");
      emitFutureEvent("toast", { message, tone: "info" });
      if (onAccepted)
        throw new Error(message);
      return;
    }
    sendingRef.current = true;
    // Resolved by `abandonSend`, raced against the pipeline below. Abandoning
    // this send cannot stop the pipeline (its invoke is already in flight), so
    // the wait has to be cut short here — otherwise a caller waiting on delivery
    // waits forever. See `abandonSend` for why that cannot be left to the
    // pipeline's own settle.
    let releaseDelivery!: () => void;
    const abandoned = new Promise<void>((resolve) => {
      releaseDelivery = resolve;
    });
    const localSend: LocalSend = { runId: null, releaseDelivery };
    localSendRef.current = localSend;

    const sendGeneration = sendGenerationRef.current + 1;
    sendGenerationRef.current = sendGeneration;
    const isCurrentSend = () => sendGenerationRef.current === sendGeneration;

    try {
      const pipeline = runSendPipeline(
        {
          isCurrentSend,
          onAccepted,
          modelId,
          onThreadActivity,
          refreshRecentRun,
          setMessages,
          thinkingLevel,
          thread,
          setRecentRun: (run) => {
            if (!isCurrentSend())
              return;
            localSend.runId = run.id;
            setRecentRun(run);
          },
        },
        payload,
      );
      // Losing the race leaves the pipeline rejected with nobody left to
      // observe it; mark that as handled so an abandoned failure is not
      // reported as an unhandled rejection. When the pipeline wins the race its
      // own rejection is what surfaces, exactly as before.
      pipeline.catch(() => {});
      await Promise.race([pipeline, abandoned]);
    }
    catch (error) {
      // Run failures are already surfaced as friendly failure bubbles by the
      // pipeline itself; what escapes to here is pre-pipeline validation (e.g.
      // attachment decode/size rejections — already user-friendly copy) or a
      // truly unexpected error. Show it verbatim — wrapping it in a "run
      // failed" template would mislabel validation feedback.
      emitFutureEvent("toast", { message: errorMessage(error), tone: "error" });
      if (onAccepted)
        throw error;
    }
    finally {
      // Release the in-flight lock — but only if a newer send/thread switch
      // hasn't already taken over (it owns the flag then).
      if (isCurrentSend()) {
        sendingRef.current = false;
        localSendRef.current = null;
      }
    }
  }, [activeRunId, modelId, onThreadActivity, refreshRecentRun, sendingRef, setMessages, setRecentRun, thinkingLevel, thread]);

  useEffect(() => {
    // Unmount (the user switched conversations — AgentThread is keyed by
    // thread id) abandons any in-flight send for this thread; let the new
    // thread's instance send freely (its run is a different session). The
    // abandoned send's stream timer is a closure-local handle now, and its
    // interval callback no-ops once `isCurrentSend()` turns false, so there's
    // nothing to clear here — it stops on its own when that send's await
    // returns.
    //
    // Deliberately does NOT release the caller waiting on delivery, unlike
    // `abandonSend`: nobody is left to release (the composer unmounted with
    // this view), and leaving the promise pending keeps the pendingPrompt
    // ownership set holding this prompt — a remounted instance of the same
    // conversation must not send its staged first message a second time.
    return () => {
      sendGenerationRef.current += 1;
      sendingRef.current = false;
      localSendRef.current = null;
    };
  }, [sendingRef]);

  // Invalidate any in-flight send this view started and release the lock.
  // Invoked by the settle watchdog once the persisted run row shows the send's
  // run is already terminal: the pipeline's invoke may never resolve (its
  // response lost to a suspended webview), so its own `finally` would never
  // run — leaving the composer locked and the streaming bubble frozen. The
  // bumped generation keeps a late-resolving pipeline from writing to the view.
  const abandonSend = useCallback(() => {
    sendGenerationRef.current += 1;
    sendingRef.current = false;
    // Release whoever is waiting on this send's delivery, so an abandoned send
    // cannot keep the composer locked on its pending send. The run is settled
    // (that is what triggered the abandon) and the watchdog is reloading its
    // persisted reply, so this reports delivery — not a rejection, which would
    // put the submitted draft back and duplicate the message the thread is
    // already showing. Dropping the whole send identity (rather than a flag)
    // also makes a second abandon of an already-abandoned send a no-op.
    const abandoned = localSendRef.current;
    localSendRef.current = null;
    abandoned?.releaseDelivery();
  }, [sendingRef]);

  return { handleSend, abandonSend, localSendRef };
}
