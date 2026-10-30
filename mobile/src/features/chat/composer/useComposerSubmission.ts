import { useFusionAvailability } from "../../../remote/useFusionAvailability";
import { fusionTask } from "@future-os/thread-projection";
import { useCallback, useLayoutEffect, useMemo, useRef, useState, type Dispatch, type SetStateAction } from "react";
import type { TFunction } from "i18next";
import type { useRemote } from "../../../remote/RemoteContext";
import type { MobileAttachment } from "../../../remote/types";
import { useCommittedRef } from "../../../lib/useCommittedRef";
import { useConversationOperation } from "../useConversationOperation";
import type { SendMessageApi } from "../useSendMessage";
import type { SkillRecommendationApi } from "../useSkillRecommendation";
import { showToast } from "../utils";

interface Options {
  remote: ReturnType<typeof useRemote>;
  t: TFunction;
  message: string;
  attachments: MobileAttachment[];
  restoring: boolean;
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
  useLayoutEffect(() => () => { attemptRef.current = null; }, [scope]);

  const submit = useCallback(async (install = false, dismiss = false) => {
    const initial = latest.current;
    // No judgment about whether the session "looks" busy: that flag is this
    // client's own projection and outlives the run it described (a missed
    // terminal event, a socket that died mid-stream), so consulting it made
    // sends vanish silently. The desktop answers for itself — a run still in
    // flight is refused there with a reason — and `attemptRef` still stops one
    // tap from submitting twice.
    if (attemptRef.current || initial.restoring || initial.remote.busy
      || initial.remote.compacting) return;
    if (!initial.message.trim() && !initial.attachments.length) return;
    if (fusionTask(initial.message) !== null && !initial.remote.capabilities.has("fusion_v1")) {showToast(initial.t("chat.fusionUnavailable"));return;}
    if (fusionTask(initial.message) !== null && !fusionAllowed.current) { showToast(initial.t("chat.fusionDisabled")); return; }
    if (fusionTask(initial.message) === "") { initial.setMessage("/fusion "); return; }
    const attempt = {};
    attemptRef.current = attempt;
    const isCurrent = captureOperation();
    const unchanged = () => isCurrent() && latest.current.message === initial.message
      && latest.current.attachments === initial.attachments;
    try {
      let composed: string | undefined;
      if (install) {
        setInstallingScope(scope);
        const result = await initial.recommendation.installAndUse(initial.message);
        if (!unchanged()) return;
        if (result === null) { showToast(initial.t("chat.skillInstallFailed")); return; }
        composed = result;
        initial.setMessage(composed);
      } else if (dismiss || initial.recommendation.suggestion) {
        initial.recommendation.dismiss();
      } else if (fusionTask(initial.message) === null) {
        const held = await initial.recommendation.evaluate(initial.message);
        if (held || !unchanged()) return;
      }
      if (!isCurrent()) return;
      initial.scrollToLatest();
      await initial.send(composed);
    } finally {
      if (attemptRef.current === attempt) {
        attemptRef.current = null;
        setInstallingScope(null);
      }
    }
  }, [captureOperation, latest, scope, fusionAllowed]);
  const sendFromComposer = useCallback(() => submit(), [submit]);
  const dismissSuggestedSkill = useCallback(() => { void submit(false, true); }, [submit]);
  const installSuggestedSkill = useCallback(() => submit(true), [submit]);
  return useMemo(() => ({
    sendFromComposer, dismissSuggestedSkill, installSuggestedSkill,
    installingSkill: installingScope === scope
  }),
    [sendFromComposer, dismissSuggestedSkill, installSuggestedSkill, installingScope, scope]);
}
