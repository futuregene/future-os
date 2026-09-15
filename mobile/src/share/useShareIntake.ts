import { useCallback, useEffect, useRef, useState } from "react";
import { AppState } from "react-native";
import { useTranslation } from "react-i18next";
import { addPendingShareListener, getPendingShare } from "future-share-intent";
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
  const readAgainRef = useRef(false);
  const importingRef = useRef(false);
  const desktopRef = useRef(remote.credentials?.expectedDesktopId);
  useEffect(() => {
    desktopRef.current = remote.credentials?.expectedDesktopId;
  }, [remote.credentials?.expectedDesktopId]);

  const intake = useCallback(async function readInbox() {
    const desktopId = remote.credentials?.expectedDesktopId;
    if (pendingRef.current || importingRef.current || !desktopId) return;
    if (readingRef.current) {
      readAgainRef.current = true;
      return;
    }
    readAgainRef.current = false;
    readingRef.current = true;
    try {
      const share = await getPendingShare();
      if (!share || desktopId !== desktopRef.current) return;
      if (share.failed) showToast(t("attachment.errors.attachment_failed"));
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
      // A native receipt may arrive after an in-flight read found no payload.
      if (readAgainRef.current && desktopId === desktopRef.current) void readInbox();
    }
  }, [remote.credentials?.expectedDesktopId, t]);

  const dismiss = () => {
    pendingRef.current = false;
    setPending(null);
  };

  // The action sheet captures this callback before dismissing. That closure
  // owns its payload even after pending is cleared for the modal transition.
  const chooseDestination = async (mode: "chat" | "workspace" | "session", destinationId?: string) => {
    if (!pending || importingRef.current) return;
    if (pending.desktopId !== desktopRef.current) return;
    if (mode === "workspace" && !remote.workspaces.some(workspace => workspace.id === destinationId)) return;
    if (mode === "session" && (!destinationId || !remote.sessions.some(session => session.sessionId === destinationId))) return;
    importingRef.current = true;
    try {
      const draftKey = desktopDraftKey(pending.desktopId, mode === "session" ? destinationId : undefined);
      const existing = await loadSessionDraft(draftKey);
      const attachments = await prepareSharedAttachments(pending.share.files, existing?.attachments ?? []);
      const text = [existing?.text.trim(), pending.share.text.trim()].filter(Boolean).join("\n\n");
      if (desktopRef.current !== pending.desktopId) return;
      await saveSessionDraft(draftKey, { text, attachments });
      if (desktopRef.current !== pending.desktopId) return;
      if (mode === "session") {
        await remote.selectSession(destinationId!);
      } else {
        await remote.newConversation(mode, destinationId);
      }
      if (desktopRef.current !== pending.desktopId) return;
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
    const receiptSubscription = addPendingShareListener(() => { void intake(); });
    return () => {
      subscription.remove();
      receiptSubscription.remove();
    };
  }, [intake, remote.credentials]);

  return { pending, dismiss, chooseDestination };
}
