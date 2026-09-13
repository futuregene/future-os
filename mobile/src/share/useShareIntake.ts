import { useCallback, useEffect, useRef, useState } from "react";
import { AppState } from "react-native";
import { useTranslation } from "react-i18next";
import { getPendingShare } from "future-share-intent";
import { showToast } from "../features/chat/utils";
import { useRemoteControls as useRemote } from "../remote/RemoteContext";
import { loadSessionDraft, saveSessionDraft } from "../remote/draftStorage";
import { prepareSharedAttachments } from "../remote/files";
import { markShareLanded } from "./shareInbox";
import { desktopDraftKey } from "../remote/desktopDraftKey";
import type { SharedContent } from "future-share-intent";

/** Read shares only after pairing, then let the user choose their destination.
 * No upload or prompt is sent until the user reviews and sends the composer. */
export function useShareIntake() {
  const remote = useRemote();
  const { t } = useTranslation();
  const [pending, setPending] = useState<{ desktopId: string; share: SharedContent } | null>(null);
  const pendingRef = useRef(false);
  const readingRef = useRef(false);
  const importingRef = useRef(false);
  const desktopRef = useRef(remote.credentials?.expectedDesktopId);
  useEffect(() => {
    desktopRef.current = remote.credentials?.expectedDesktopId;
  }, [remote.credentials?.expectedDesktopId]);

  const intake = useCallback(async () => {
    const desktopId = remote.credentials?.expectedDesktopId;
    if (readingRef.current || pendingRef.current || importingRef.current || !desktopId) return;
    readingRef.current = true;
    try {
      const share = await getPendingShare();
      if (!share) return;
      if (!share.text.trim() && share.files.length === 0) {
        if (share.tooLarge) showToast(t("attachment.errors.attachment_file_too_large"));
        return;
      }
      pendingRef.current = true;
      setPending({ desktopId, share });
    } catch {
      showToast(t("attachment.errors.attachment_failed"));
    } finally {
      readingRef.current = false;
    }
  }, [remote.credentials?.expectedDesktopId, t]);

  const dismiss = () => {
    pendingRef.current = false;
    setPending(null);
  };

  // The action sheet captures this callback before dismissing. That closure
  // owns its payload even after pending is cleared for the modal transition.
  const chooseDestination = async (mode: "chat" | "workspace", workspaceId?: string) => {
    if (!pending || importingRef.current) return;
    if (pending.desktopId !== desktopRef.current) return;
    if (mode === "workspace" && !remote.workspaces.some(workspace => workspace.id === workspaceId)) return;
    importingRef.current = true;
    try {
      const draftKey = desktopDraftKey(pending.desktopId);
      const existing = await loadSessionDraft(draftKey);
      const attachments = await prepareSharedAttachments(pending.share.files, existing?.attachments ?? []);
      const text = [existing?.text.trim(), pending.share.text.trim()].filter(Boolean).join("\n\n");
      await saveSessionDraft(draftKey, { text, attachments });
      if (desktopRef.current !== pending.desktopId) return;
      await remote.newConversation(mode, workspaceId);
      markShareLanded();
      if (pending.share.tooLarge) showToast(t("attachment.errors.attachment_file_too_large"));
    } catch (error) {
      const key = error instanceof Error ? error.message : "attachment_failed";
      showToast(t(`attachment.errors.${key}`));
    } finally {
      importingRef.current = false;
    }
  };

  useEffect(() => {
    if (!remote.credentials) return;
    // Read the native inbox after the initial render; foreground receipts use
    // the same path, and the in-flight guard deduplicates concurrent reads.
    void Promise.resolve().then(intake);
    const subscription = AppState.addEventListener("change", state => {
      if (state === "active") void intake();
    });
    return () => subscription.remove();
  }, [intake, remote.credentials]);

  return { pending, dismiss, chooseDestination };
}
