import { useCallback, useEffect, useRef, useState, type Dispatch, type SetStateAction } from "react";
import { AppState, Platform } from "react-native";
import type { TFunction } from "i18next";
import { useRemote } from "../../remote/RemoteContext";
import { flushSessionDraft, loadSessionDraft, scheduleSessionDraft } from "../../remote/draftStorage";
import { desktopDraftKey } from "../../remote/desktopDraftKey";
import { recoverPendingImagePickerAttachments } from "../../remote/files";
import type { MobileAttachment, SessionReferenceMap, SessionReferenceTarget } from "../../remote/types";
import { pruneSessionReferences } from "./sessionCompletion";
import { showToast } from "./utils";

type Remote = ReturnType<typeof useRemote>;

export interface ComposerDraftApi {
  message: string;
  setMessage: Dispatch<SetStateAction<string>>;
  attachments: MobileAttachment[];
  setAttachments: Dispatch<SetStateAction<MobileAttachment[]>>;
  /**
   * The conversations the draft's `#title` tokens stand for. The composer shows
   * the token (a TextInput cannot style part of its text the way the desktop's
   * contentEditable pill does) and the send expands it back into the link — the
   * id has to be kept somewhere until then.
   */
  sessionRefs: SessionReferenceMap;
  rememberSessionRef: (token: string, target: SessionReferenceTarget) => void;
}

export function useComposerDraft(remote: Remote, t: TFunction): ComposerDraftApi {
  const [message, setMessage] = useState("");
  const [attachments, setAttachments] = useState<MobileAttachment[]>([]);
  const [sessionRefs, setSessionRefs] = useState<SessionReferenceMap>({});
  // A ref, not the state value, so `rememberSessionRef` can stay a stable
  // callback while still pruning against the current text. Written in an effect
  // (writing a ref during render is what the lint rule forbids); a pick happens
  // in an event handler, after that effect has run.
  const messageRef = useRef("");
  useEffect(() => {
    messageRef.current = message;
  }, [message]);
  // Per-session composer draft: the unsent text/attachments survive leaving the
  // screen and coming back (G6). The draft conversation (no session yet) uses a
  // fixed key so a re-created new-conversation draft restores what was started.
  const draftKey = desktopDraftKey(remote.credentials!.expectedDesktopId, remote.selectedSessionId);
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
        setSessionRefs(draft?.refs ?? {});
        restoringDraftRef.current = false;
      }
    })();
  }, [draftKey, t]);

  // Persist edits. The restore-driven update is skipped (the effect above
  // already loaded the draft), and temporary camera/cache files are released
  // when removed so they don't accumulate on disk. References the text no
  // longer mentions are left out, so the stored draft does not grow with
  // conversations no longer referenced (including after a send clears the
  // text).
  useEffect(() => {
    if (restoringDraftRef.current) return;
    scheduleSessionDraft(draftKey, {
      text: message,
      attachments,
      refs: pruneSessionReferences(sessionRefs, message),
    });
  }, [attachments, draftKey, message, sessionRefs]);

  useEffect(() => {
    const subscription = AppState.addEventListener("change", state => {
      if (state !== "active") void flushSessionDraft(draftKey);
    });
    return () => { subscription.remove(); void flushSessionDraft(draftKey); };
  }, [draftKey]);

  const rememberSessionRef = useCallback((token: string, target: SessionReferenceTarget) => {
    // Pruning here (rather than in an effect on `message`) keeps the map from
    // filling with tokens the user has since deleted; the latest text is read
    // through a ref so the callback stays stable.
    setSessionRefs(current => ({
      ...pruneSessionReferences(current, messageRef.current),
      [token]: target,
    }));
  }, []);

  return { message, setMessage, attachments, setAttachments, sessionRefs, rememberSessionRef };
}
