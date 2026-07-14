import type { Dispatch, MutableRefObject, SetStateAction } from "react";
import type { StoredRun, StoredThread } from "../../integrations/storage/threadStore";
import type { AgentMessage } from "./agentThreadTypes";
import type { ComposerSendPayload } from "./Composer";
import { useCallback, useEffect, useRef } from "react";
import i18n from "../../i18n";
import { errorMessage } from "../../lib/errors";
import { emitFutureEvent } from "../../lib/futureEvents";
import { runSendPipeline } from "./sendPipeline";

interface UseSendMessageInput {
  thread: StoredThread | null;
  threadId: string | null;
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
  refreshRecentRun: (threadId: string, workspaceId?: string | null) => Promise<void>;
  onThreadActivity: () => void;
}

/**
 * The send entry point. Guards one prompt at a time per session, stamps a send
 * generation so a superseded send stops mutating the view, then delegates the
 * heavy lifting to {@link runSendPipeline}. Releasing the lock only when this
 * send still owns it keeps a newer send/thread switch in control of the flag.
 */
export function useSendMessage({
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
}: UseSendMessageInput) {
  const sendGenerationRef = useRef(0);

  const handleSend = useCallback(async (payload: ComposerSendPayload) => {
    if (!thread)
      return;
    // One prompt at a time per session. `sendingRef` guards a send this
    // view started; `activeRunId` guards a run already in flight that this view
    // is only re-attached to (backgrounded, reloaded, or remote-driven). Every
    // send path — composer, `recover-run`, pendingPrompt — funnels through here,
    // so a second prompt while one runs would be rejected by the agent
    // ("already running") and leave a stray failed run. Reject it up front, with
    // visible feedback instead of a silent drop.
    if (sendingRef.current || activeRunId) {
      emitFutureEvent("toast", { message: i18n.t("agent:thread.alreadyRunning"), tone: "info" });
      return;
    }
    sendingRef.current = true;

    const sendGeneration = sendGenerationRef.current + 1;
    sendGenerationRef.current = sendGeneration;
    const isCurrentSend = () => sendGenerationRef.current === sendGeneration;

    try {
      await runSendPipeline(
        { isCurrentSend, modelId, onThreadActivity, refreshRecentRun, setMessages, setRecentRun, thinkingLevel, thread },
        payload,
      );
    }
    catch (error) {
      emitFutureEvent("toast", { message: errorMessage(error), tone: "error" });
      throw error;
    }
    finally {
      // Release the in-flight lock — but only if a newer send/thread switch
      // hasn't already taken over (it owns the flag then).
      if (isCurrentSend())
        sendingRef.current = false;
    }
  }, [activeRunId, modelId, onThreadActivity, refreshRecentRun, sendingRef, setMessages, setRecentRun, thinkingLevel, thread]);

  useEffect(() => {
    return () => {
      sendGenerationRef.current += 1;
      // A switch away abandons any in-flight send for this thread; let the new
      // thread send freely (its run is a different session). The abandoned send's
      // stream timer is a closure-local handle now, and its interval callback
      // no-ops once `isCurrentSend()` turns false, so there's nothing to clear
      // here — it stops on its own when that send's await returns.
      sendingRef.current = false;
    };
  }, [sendingRef, threadId]);

  return handleSend;
}
