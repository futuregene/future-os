import { useFusionAvailability } from "../../../remote/useFusionAvailability";
import { fusionTask } from "@future-os/thread-projection";
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, type Dispatch, type SetStateAction } from "react";
import type { TFunction } from "i18next";
import type { useRemote } from "../../../remote/RemoteContext";
import type { MobileAttachment } from "../../../remote/types";
import { useCommittedRef } from "../../../lib/useCommittedRef";
import { useConversationOperation } from "../useConversationOperation";
import type { SendMessageApi, SendMessageIntent } from "../useSendMessage";
import { answerOrNull, choiceAnswerText, type ChoiceAnswer } from "../choiceAnswer";
import type { SkillRecommendationApi } from "../useSkillRecommendation";
import { showToast } from "../utils";

interface Options {
  remote: ReturnType<typeof useRemote>;
  t: TFunction;
  message: string;
  attachments: MobileAttachment[];
  /**
   * The draft's answer to a `futureos-choice` card, if any.
   *
   * Read here for two reasons: it counts as something to send (the phone's field
   * cannot hold it, §5.7.1), and sending must attribute it (§7) — the prompt the
   * model reads is `[问题 → 答案]` followed by whatever was typed.
   */
  choice: ChoiceAnswer | null;
  /** Drop the answer once the send it belonged to was taken. */
  clearChoice: () => void;
  restoring: boolean;
  /**
   * Whether this conversation is compacting right now.
   *
   * Passed in rather than read off `remote` because a manual compaction the reader
   * just started is this screen's own attempt (`useCompactContext.pending`) before
   * the Agent's flag catches up; the composer's send gate already reads the two as
   * one value, and the hold below must agree with it. Defaults to the Agent's flag.
   */
  compacting?: boolean;
  setMessage: Dispatch<SetStateAction<string>>;
  send: SendMessageApi["send"];
  recommendation: SkillRecommendationApi;
  scrollToLatest: () => void;
}

/** Own a submit from evaluation through delivery, tied to one navigation intent. */
export function useComposerSubmission(options: Options) {
  const fusionAvailable = useFusionAvailability();
  const fusionAllowed = useCommittedRef(fusionAvailable);
  const latest = useCommittedRef(options);
  const { scope, captureOperation } = useConversationOperation(options.remote);
  const attemptRef = useRef<object | null>(null);
  const [installingScope, setInstallingScope] = useState<object | null>(null);
  const [holdingScope, setHoldingScope] = useState<object | null>(null);
  /**
   * Submits whose tap landed while the context was compacting.
   *
   * A compaction owns the session for as long as the summary takes — minutes on
   * a long conversation — and the Agent refuses prompts meanwhile (`session_busy`
   * with `busy_reason: compaction`). The phone used to disable the send button
   * for that whole window, so a message written during a compaction could not be
   * handed over at all. It now keeps the draft and waits here instead: the tap is
   * honoured, the words stay visible in the field, and the send goes the moment
   * the Agent closes the fence.
   */
  const compactionWaitersRef = useRef(new Set<(ended: boolean) => void>());
  const resolveCompactionWaiters = useCallback((ended: boolean) => {
    const waiters = Array.from(compactionWaitersRef.current);
    compactionWaitersRef.current.clear();
    for (const resume of waiters) resume(ended);
  }, []);
  // Leaving the screen ends the wait without delivering: the draft is persisted
  // where it was written, and coming back to this conversation does not send a
  // message the reader has long stopped watching. `captureOperation` would refuse
  // the delivery anyway; this only stops the promise from hanging.
  useEffect(() => () => resolveCompactionWaiters(false), [resolveCompactionWaiters]);
  const compacting = options.compacting ?? options.remote.compacting;
  const compactingRef = useCommittedRef(compacting);
  useEffect(() => {
    if (!compacting) resolveCompactionWaiters(true);
  }, [compacting, resolveCompactionWaiters]);
  const waitForCompactionEnd = useCallback(() => new Promise<boolean>((resolve) => {
    compactionWaitersRef.current.add(resolve);
  }), []);
  useLayoutEffect(() => () => { attemptRef.current = null; }, [scope]);

  const submit = useCallback(async (
    install = false,
    dismiss = false,
    intent?: SendMessageIntent,
  ): Promise<boolean> => {
    const initial = latest.current;
    // No judgment about whether the session "looks" busy: that flag is this
    // client's own projection and outlives the run it described (a missed
    // terminal event, a socket that died mid-stream), so consulting it made
    // sends vanish silently. The desktop answers for itself — a run still in
    // flight is refused there with a reason — and `attemptRef` still stops one
    // tap from submitting twice.
    if (attemptRef.current || initial.restoring || initial.remote.busy) return false;
    if (!initial.message.trim() && !initial.attachments.length && !answerOrNull(initial.choice)) return false;
    if (fusionTask(initial.message) !== null && !initial.remote.capabilities.has("fusion_v1")) {showToast(initial.t("chat.fusionUnavailable"));return false;}
    if (fusionTask(initial.message) !== null && !fusionAllowed.current) { showToast(initial.t("chat.fusionDisabled")); return false; }
    if (fusionTask(initial.message) === "") { initial.setMessage("/fusion "); return false; }
    const attempt = {};
    attemptRef.current = attempt;
    const isCurrent = captureOperation();
    const unchanged = () => isCurrent() && latest.current.message === initial.message
      && latest.current.attachments === initial.attachments;
    try {
      if (compactingRef.current) {
        setHoldingScope(scope);
        const ended = await waitForCompactionEnd();
        if (attemptRef.current === attempt) setHoldingScope(null);
        // The compaction that ended may have been followed by another (or the
        // screen may have moved on): the send is refused exactly as it was
        // before, and the draft stays where the reader can see it.
        if (!ended || !isCurrent() || !unchanged()) return false;
        if (compactingRef.current) return false;
      }
      let composed: string | undefined;
      if (install) {
        setInstallingScope(scope);
        const result = await initial.recommendation.installAndUse(initial.message);
        if (!unchanged()) return false;
        if (result === null) { showToast(initial.t("chat.skillInstallFailed")); return false; }
        composed = result;
        initial.setMessage(composed);
      } else if (dismiss || initial.recommendation.suggestion) {
        initial.recommendation.dismiss();
      } else if (fusionTask(initial.message) === null) {
        const held = await initial.recommendation.evaluate(initial.message);
        if (held || !unchanged()) return false;
      }
      if (!isCurrent()) return false;
      initial.scrollToLatest();
      // The answer travels as text (§7), prefixed to whatever was typed — unless
      // this submit composed its own text (a skill command), which is sent as the
      // typed value it already is.
      const body = composed ?? initial.message;
      const override = initial.choice ? withChoiceAnswer(initial.choice, body) : composed;
      // Whether the session took it: an interrupt that ends the run afterwards
      // must not do so on the strength of a send that only put the draft back.
      const delivered = intent ? await initial.send(override, intent) : await initial.send(override);
      // A taken send is the answer's end: the draft's text is cleared by the send
      // itself, so the chip goes with it. A refused one keeps both.
      if (delivered)
        initial.clearChoice();
      return delivered;
    } finally {
      if (attemptRef.current === attempt) {
        attemptRef.current = null;
        setInstallingScope(null);
        setHoldingScope(null);
      }
    }
  }, [captureOperation, latest, scope, fusionAllowed, waitForCompactionEnd, compactingRef]);
  /**
   * The composer's send, with what this send means while the reply streams.
   *
   * `intent` names the queueing policy for the sends whose event is "hand this
   * to the session" (the send sheet's two rows), so their delivery does not
   * depend on this screen's reading of the run.
   */
  const sendFromComposer = useCallback(
    (intent?: SendMessageIntent) => submit(false, false, intent),
    [submit],
  );
  const dismissSuggestedSkill = useCallback(() => { void submit(false, true); }, [submit]);
  const installSuggestedSkill = useCallback(() => submit(true), [submit]);
  return useMemo(() => ({
    sendFromComposer, dismissSuggestedSkill, installSuggestedSkill,
    installingSkill: installingScope === scope,
    /** The tap was taken and waits for the compaction to finish (§ below). */
    holdingSend: holdingScope === scope,
  }),
    [sendFromComposer, dismissSuggestedSkill, installSuggestedSkill, installingScope, holdingScope, scope]);
}

/**
 * The text a send carries when the draft holds a card's answer (§7):
 * `[接下来做哪一件？ → 重画森林图]`, then the reader's own words after it.
 *
 * Plain text, so the answer still reads on a TUI, an IM bot or a message pasted
 * elsewhere — which is why the answer is not a `futureos://` link.
 *
 * An answer with nothing ticked contributes no tag at all: §7's sentence is
 * `[question → answer]`, so printing one for a question with no pick would hand
 * the agent a question it reads as an answer. `answerOrNull` drops such an answer
 * at the draft already; this is the same rule at the last gate, where every send
 * passes.
 */
function withChoiceAnswer(answer: ChoiceAnswer, text: string): string {
  const body = text.trim();
  if (!answerOrNull(answer))
    return body;
  return body ? `${choiceAnswerText(answer)} ${body}` : choiceAnswerText(answer);
}
