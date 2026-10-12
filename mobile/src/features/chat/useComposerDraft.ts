import { useCallback, useEffect, useLayoutEffect, useRef, useState, type Dispatch, type SetStateAction } from "react";
import { AppState, Platform } from "react-native";
import type { TFunction } from "i18next";
import type { useRemote } from "../../remote/RemoteContext";
import { useCommittedRef } from "../../lib/useCommittedRef";
import { flushSessionDraft, loadSessionDraft, scheduleSessionDraft } from "../../remote/draftStorage";
import { desktopDraftKey } from "../../remote/desktopDraftKey";
import { deleteTemporaryAttachment, recoverPendingImagePickerAttachments } from "../../remote/files";
import type { MobileAttachment, SessionReferenceMap, SessionReferenceTarget } from "../../remote/types";
import { pruneSessionReferences } from "./sessionCompletion";
import { showToast } from "./utils";

type Remote = ReturnType<typeof useRemote>;
export interface ComposerDraftApi {
  message: string;
  setMessage: Dispatch<SetStateAction<string>>;
  attachments: MobileAttachment[];
  setAttachments: Dispatch<SetStateAction<MobileAttachment[]>>;
  sessionRefs: SessionReferenceMap;
  rememberSessionRef: (token: string, target: SessionReferenceTarget) => void;
  restoring: boolean;
}
interface DraftState {
  key: string;
  message: string;
  attachments: MobileAttachment[];
  sessionRefs: SessionReferenceMap;
  restoring: boolean;
}
const emptyDraft = (key: string): DraftState => ({ key, message: "", attachments: [], sessionRefs: {}, restoring: true });

export function useComposerDraft(remote: Remote, t: TFunction): ComposerDraftApi {
  const draftKey = desktopDraftKey(remote.credentials!.expectedDesktopId, remote.selectedSessionId);
  const [state, setState] = useState(() => emptyDraft(draftKey));
  const tRef = useCommittedRef(t);
  const activeKeyRef = useRef(draftKey);
  const loadRef = useRef<object | null>(null);
  const editRevisionRef = useRef(0);
  useLayoutEffect(() => {
    activeKeyRef.current = draftKey;
    const operation = {};
    loadRef.current = operation;
    const revision = editRevisionRef.current;
    // A new draft must not expose the outgoing conversation's text while loading.
    // eslint-disable-next-line react-hooks/set-state-in-effect
    setState(current => current.key === draftKey ? current : emptyDraft(draftKey));
    const isCurrent = () => loadRef.current === operation && editRevisionRef.current === revision;
    void (async () => {
      const draft = await loadSessionDraft(draftKey);
      if (!isCurrent()) return;
      let attachments = draft?.attachments ?? [];
      if (Platform.OS === "android") {
        try { attachments = await recoverPendingImagePickerAttachments(attachments); }
        catch (error) {
          if (isCurrent()) {
            const key = error instanceof Error ? error.message : "attachment_failed";
            showToast(tRef.current(`attachment.errors.${key}`));
          }
        }
      }
      if (!isCurrent()) {
        const original = new Set((draft?.attachments ?? []).map(item => item.localUri));
        for (const item of attachments) {
          if (!original.has(item.localUri) && item.temporary) {
            try { deleteTemporaryAttachment(item); } catch { /* best-effort cleanup */ }
          }
        }
        return;
      }
      setState({
        key: draftKey, message: draft?.text ?? "", attachments,
        sessionRefs: draft?.refs ?? {}, restoring: false
      });
    })();
    return () => { if (loadRef.current === operation) loadRef.current = null; };
  }, [draftKey, tRef]);

  const setMessage = useCallback<Dispatch<SetStateAction<string>>>(value => {
    if (activeKeyRef.current !== draftKey) return;
    editRevisionRef.current += 1;
    setState(current => current.key !== draftKey ? current : {
      ...current, message: typeof value === "function" ? value(current.message) : value, restoring: false,
    });
  }, [draftKey]);
  const setAttachments = useCallback<Dispatch<SetStateAction<MobileAttachment[]>>>(value => {
    if (activeKeyRef.current !== draftKey) return;
    editRevisionRef.current += 1;
    setState(current => current.key !== draftKey ? current : {
      ...current, attachments: typeof value === "function" ? value(current.attachments) : value, restoring: false,
    });
  }, [draftKey]);
  const rememberSessionRef = useCallback((token: string, target: SessionReferenceTarget) => {
    if (activeKeyRef.current !== draftKey) return;
    editRevisionRef.current += 1;
    setState(current => current.key !== draftKey ? current : {
      ...current,
      sessionRefs: { ...pruneSessionReferences(current.sessionRefs, current.message), [token]: target },
      restoring: false,
    });
  }, [draftKey]);

  useEffect(() => {
    if (state.key !== draftKey || state.restoring) return;
    scheduleSessionDraft(draftKey, {
      text: state.message, attachments: state.attachments,
      refs: pruneSessionReferences(state.sessionRefs, state.message)
    });
  }, [draftKey, state]);
  useEffect(() => {
    const subscription = AppState.addEventListener("change", next => {
      if (next !== "active") void flushSessionDraft(draftKey);
    });
    return () => { subscription.remove(); void flushSessionDraft(draftKey); };
  }, [draftKey]);
  const current = state.key === draftKey ? state : emptyDraft(draftKey);
  return { ...current, setMessage, setAttachments, rememberSessionRef };
}
