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
  const [pending, setPending] = useState<SharedContent | null>(null);
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
      setPending(share);
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

  /**
   * Land the share where the user said, on the machine they named.
   *
   * The share arrived on the *phone* and belongs to no machine, so the machine
   * is the sheet's to name rather than the one that happened to be open when the
   * payload was read — and it need not be the machine the phone is reading now.
   * What the write needs is that the machine still takes commands: a draft
   * written for a machine that went away is content addressed to nobody, and a
   * share marked landed without a conversation behind it is a share thrown away.
   *
   * The action sheet captures this callback before dismissing. That closure owns
   * its payload even after pending is cleared for the modal transition.
   */
  const chooseDestination = async (
    desktopId: string,
    mode: "chat" | "workspace" | "session",
    destinationId?: string,
  ) => {
    if (!pending || importingRef.current) return;
    // Read live, never from the render that drew the sheet: the machine has to
    // answer *now*, whichever one the user named.
    if (!remote.catalogs.commandsFor(desktopId)) return;
    const sessions = desktopId === remote.credentials?.expectedDesktopId
      ? remote.sessions
      : remote.catalogs.catalogs.find(entry => entry.desktopId === desktopId)?.sessions ?? [];
    if (mode === "session" && !sessions.some(session => session.sessionId === destinationId)) return;
    // The open machine's workspaces are here to check against; a peer's live in
    // the sheet that read them, so the row the user tapped is that machine's
    // own answer.
    if (mode === "workspace"
      && desktopId === remote.credentials?.expectedDesktopId
      && !remote.workspaces.some(workspace => workspace.id === destinationId)) return;
    importingRef.current = true;
    try {
      const draftKey = desktopDraftKey(desktopId, mode === "session" ? destinationId : undefined);
      const existing = await loadSessionDraft(draftKey);
      const attachments = await prepareSharedAttachments(pending.files, existing?.attachments ?? []);
      const text = [existing?.text.trim(), pending.text.trim()].filter(Boolean).join("\n\n");
      if (!remote.catalogs.commandsFor(desktopId)) return;
      await saveSessionDraft(draftKey, { text, attachments });
      if (!remote.catalogs.commandsFor(desktopId)) return;
      // Both verbs name the machine, so a phone that moved while the draft was
      // being staged still opens the conversation on the machine asked for.
      if (mode === "session") await remote.openSessionOnDesktop(desktopId, destinationId ?? "");
      else await remote.newConversationOn(desktopId, mode, destinationId);
      markShareLanded();
      if (pending.tooLarge) showToast(t("attachment.errors.attachment_file_too_large"));
    } catch (error) {
      const key = error instanceof Error && error.message === "desktop_unavailable"
        ? "desktops.switchFailed"
        : `attachment.errors.${error instanceof Error ? error.message : "attachment_failed"}`;
      showToast(t(key));
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
