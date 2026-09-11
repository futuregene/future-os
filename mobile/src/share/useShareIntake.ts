import { useCallback, useEffect, useRef } from "react";
import { AppState } from "react-native";
import { useTranslation } from "react-i18next";
import { getPendingShare } from "future-share-intent";
import { showToast } from "../features/chat/utils";
import { useRemote } from "../remote/RemoteContext";
import {
  loadSessionDraft,
  NEW_CONVERSATION_DRAFT_KEY,
  saveSessionDraft,
} from "../remote/draftStorage";
import { prepareSharedAttachments } from "../remote/files";
import { markShareLanded } from "./shareInbox";

/**
 * Move content shared from another app into a new conversation's composer.
 *
 * Runs on mount and on every return to the foreground: a share either starts
 * the app (the share intent is the launch intent) or resumes it, where
 * `MainActivity` is `singleTask` and the payload arrives through `onNewIntent`.
 * The native side hands the payload over exactly once, so re-checking on every
 * foreground is free when nothing was shared.
 */
export function useShareIntake(): void {
  const remote = useRemote();
  const { t } = useTranslation();
  const readingRef = useRef(false);

  const intake = useCallback(async () => {
    if (readingRef.current) return;
    readingRef.current = true;
    try {
      const share = await getPendingShare();
      if (!share) return;
      const attachments = share.files.length ? await prepareSharedAttachments(share.files) : [];
      const text = share.text.trim();
      if (!text && attachments.length === 0) {
        if (share.tooLarge) showToast(t("attachment.errors.attachment_file_too_large"));
        return;
      }
      // Stage into the composer's new-conversation draft instead of pushing
      // state around: the composer already restores that slot on mount, so the
      // content is reviewable before the user sends it, and anything already
      // typed there is kept.
      const existing = await loadSessionDraft(NEW_CONVERSATION_DRAFT_KEY);
      await saveSessionDraft(NEW_CONVERSATION_DRAFT_KEY, {
        text: [existing?.text.trim(), text].filter(Boolean).join("\n\n"),
        attachments: [...(existing?.attachments ?? []), ...attachments],
      });
      // A share starts its own conversation rather than landing in whatever the
      // app happened to be showing; the previous composer keeps its draft.
      await remote.newConversation("chat");
      // Tell the chat screen to re-read that draft (see shareInbox).
      markShareLanded();
      if (share.tooLarge) showToast(t("attachment.errors.attachment_file_too_large"));
    } catch (error) {
      const key = error instanceof Error ? error.message : "attachment_failed";
      showToast(t(`attachment.errors.${key}`));
    } finally {
      readingRef.current = false;
    }
  }, [remote, t]);

  useEffect(() => {
    // A share is readable only once and the native side consumes it on read, so
    // wait until there is a paired desktop to open a conversation against —
    // otherwise a share that arrives while unpaired would be dropped instead of
    // waiting for the user to finish pairing.
    if (!remote.credentials) return;
    void intake();
    const subscription = AppState.addEventListener("change", state => {
      if (state === "active") void intake();
    });
    return () => subscription.remove();
  }, [intake, remote.credentials]);
}
