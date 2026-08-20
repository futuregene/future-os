import { useEffect, useRef, useState, type Dispatch, type SetStateAction } from "react";
import { Platform } from "react-native";
import type { TFunction } from "i18next";
import { useRemote } from "../../remote/RemoteContext";
import { loadSessionDraft, saveSessionDraft } from "../../remote/draftStorage";
import { recoverPendingImagePickerAttachments } from "../../remote/files";
import type { MobileAttachment } from "../../remote/types";
import { showToast } from "./utils";

type Remote = ReturnType<typeof useRemote>;

export interface ComposerDraftApi {
  message: string;
  setMessage: Dispatch<SetStateAction<string>>;
  attachments: MobileAttachment[];
  setAttachments: Dispatch<SetStateAction<MobileAttachment[]>>;
}

export function useComposerDraft(remote: Remote, t: TFunction): ComposerDraftApi {
  const [message, setMessage] = useState("");
  const [attachments, setAttachments] = useState<MobileAttachment[]>([]);
  // Per-session composer draft: the unsent text/attachments survive leaving the
  // screen and coming back (G6). The draft conversation (no session yet) uses a
  // fixed key so a re-created new-conversation draft restores what was started.
  const draftKey = remote.selectedSessionId || "draft:new";
  const restoringDraftRef = useRef(false);
  const activeDraftKeyRef = useRef(draftKey);

  // Restore this conversation's draft when the screen (re)opens. Uses the
  // sessionId captured at mount — the draft conversation key stays stable
  // across the "" placeholder, and a bound session keeps its own slot.
  useEffect(() => {
    restoringDraftRef.current = true;
    activeDraftKeyRef.current = draftKey;
    const key = draftKey;
    void (async () => {
      const draft = await loadSessionDraft(key);
      let restoredAttachments = draft?.attachments ?? [];
      if (Platform.OS === "android") {
        try {
          restoredAttachments = await recoverPendingImagePickerAttachments(restoredAttachments);
        } catch (error) {
          const errorKey = error instanceof Error ? error.message : "attachment_failed";
          showToast(t(`attachment.errors.${errorKey}`));
        }
      }
      // Guard against a conversation switch racing the async load or Android
      // pending-result recovery after MainActivity reconstruction.
      if (restoringDraftRef.current && activeDraftKeyRef.current === key) {
        setMessage(draft?.text ?? "");
        setAttachments(restoredAttachments);
        restoringDraftRef.current = false;
      }
    })();
  }, [draftKey, t]);

  // Persist edits. The restore-driven update is skipped (the effect above
  // already loaded the draft), and temporary camera/cache files are released
  // when removed so they don't accumulate on disk.
  useEffect(() => {
    if (restoringDraftRef.current) return;
    void saveSessionDraft(draftKey, { text: message, attachments });
  }, [attachments, draftKey, message]);

  return { message, setMessage, attachments, setAttachments };
}
