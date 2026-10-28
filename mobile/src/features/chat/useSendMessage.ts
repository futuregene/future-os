import { useCallback, useMemo, type Dispatch, type SetStateAction } from "react";
import type { TFunction } from "i18next";
import { File } from "expo-file-system";
import { mimeFor } from "../../remote/files";
import { useRemote } from "../../remote/RemoteContext";
import type { MobileAttachment, SessionReferenceMap, TimelineItem } from "../../remote/types";
import { expandSessionReferences } from "./sessionCompletion";
import { useCommittedRef } from "../../lib/useCommittedRef";
import { useConversationOperation } from "./useConversationOperation";
import { showToast } from "./utils";

type Remote = ReturnType<typeof useRemote>;

export interface SendMessageApi {
  /**
   * Send the composer's message. `override` sends an explicit text instead —
   * used after composing a skill command, where the value is not yet in state
   * (reading `message` in the same tick would send the pre-composition text).
   */
  send: (override?: string) => Promise<void>;
  retryMessage: (item: TimelineItem) => void;
  continueMessage: (item: TimelineItem) => void;
  /** Fork the conversation at a settled reply's turn and open the child. */
  forkMessage: (item: TimelineItem) => void;
}

export interface SendMessageOptions {
  remote: Remote;
  t: TFunction;
  message: string;
  attachments: MobileAttachment[];
  setMessage: Dispatch<SetStateAction<string>>;
  setAttachments: Dispatch<SetStateAction<MobileAttachment[]>>;
  setTransferProgress: (value: number | null) => void;
  sessionRefs?: SessionReferenceMap;
  compactionPending?: boolean;
}

export function useSendMessage(options: SendMessageOptions): SendMessageApi {
  const inputRef = useCommittedRef(options);
  const { captureOperation } = useConversationOperation(options.remote);
  const send = useCallback(async (override?: string) => {
    const { remote, t, message, attachments, setMessage, setAttachments, setTransferProgress,
      sessionRefs = {}, compactionPending = false } = inputRef.current;
    if (compactionPending || remote.compacting) { showToast(t("chat.compacting")); return; }
    const draftValue = (override ?? message).trim();
    const value = expandSessionReferences(draftValue, sessionRefs);
    if (!value && !attachments.length) return;
    const isCurrent = captureOperation();
    const sameText = () => [message.trim(), draftValue].includes(inputRef.current.message.trim());
    setTransferProgress(attachments.length ? 0 : null);
    try {
      await remote.sendMessage(value, attachments, (done, total) => {
        if (isCurrent()) setTransferProgress(total > 0 ? done / total : null);
      });
      if (!isCurrent()) return;
      if (sameText()) setMessage("");
      const submitted = new Set(attachments.map(item => item.localUri));
      setAttachments(inputRef.current.attachments.filter(item => !submitted.has(item.localUri)));
    } catch (error) {
      if (!isCurrent()) return;
      if (sameText()) setMessage(draftValue);
      const key = error instanceof Error ? error.message : "";
      // `superseded` is the desktop refusing because another device holds it.
      // The composer's own notice says so while the pointer already reads
      // standby; this branch is the race — control moved a moment ago — and
      // "message failed to send" there leaves the user with no cause at all.
      const holder = remote.presence?.holder?.name?.trim();
      showToast(key === "superseded"
        ? (holder
            ? t("chat.standbyCannotSend", { name: holder })
            : t("chat.standbyCannotSendUnknown"))
        : key === "send_compacting" ? t("chat.compacting")
          : key === "prompt_too_large" ? t("chat.promptTooLarge")
            : key.includes("session_not_on_desktop") ? t("chat.sessionNotOnDesktop")
              : t("chat.sendFailed"));
    } finally {
      if (isCurrent()) setTransferProgress(null);
    }
  }, [captureOperation, inputRef]);

  const retryMessage = useCallback(
    (item: TimelineItem) => {
      const { remote, t, compactionPending } = inputRef.current;
      const compacting = compactionPending || remote.compacting;
      const isCurrent = captureOperation();
      if (compacting) { showToast(t("chat.compacting")); return; }
      if (item.kind !== "message" || item.role !== "assistant") return;
      const items = remote.timeline.items;
      const index = items.findIndex(entry => entry.id === item.id);
      for (let i = index - 1; i >= 0; i -= 1) {
        const prev = items[i];
        if (prev?.kind === "message" && prev.role === "user") {
          void (async () => {
            const retryAttachments: MobileAttachment[] = [];
            for (const attachment of prev.attachments ?? []) {
              if (!isCurrent()) return;
              if (/^[a-z][a-z0-9+.-]*:\/\//i.test(attachment.path)) {
                const file = new File(attachment.path);
                retryAttachments.push({
                  localUri: file.uri,
                  name: attachment.name,
                  mimeType: mimeFor(attachment.name),
                  kind: attachment.kind === "image" ? "image" : "file",
                  originalSize: file.size,
                  transferSize: file.size,
                  mobilePreviewUnsupported: attachment.mobilePreviewUnsupported,
                });
                continue;
              }
              const info = await remote.prepareAttachment(attachment);
              const cached = remote.cachedAttachment(attachment);
              const file = cached?.file ?? (await remote.downloadAttachment(info));
              retryAttachments.push({
                localUri: file.uri,
                name: attachment.name,
                transferName: info.name,
                mimeType: info.mimeType,
                kind: attachment.kind === "image" ? "image" : "file",
                originalSize: info.size,
                transferSize: info.size,
                mobilePreviewUnsupported: attachment.mobilePreviewUnsupported,
              });
            }
            if (isCurrent()) await remote.sendMessage(prev.text, retryAttachments);
          })().catch(() => { if (isCurrent()) showToast(t("chat.sendFailed")); });
          return;
        }
      }
    },
    [captureOperation, inputRef],
  );

  const continueMessage = useCallback(
    (item: TimelineItem) => {
      const { remote, t, compactionPending } = inputRef.current;
      const compacting = compactionPending || remote.compacting;
      const isCurrent = captureOperation();
      if (compacting) { showToast(t("chat.compacting")); return; }
      if (item.kind !== "message" || item.role !== "assistant" || !item.runId) return;
      void remote
        .continueRun(remote.selectedSessionId, item.runId)
        .catch((error: unknown) => {
          if (!isCurrent()) return;
          // The desktop prefixes a refused continuation with a stable token
          // (`continue_unavailable`): it received the tap and said no — the
          // reply finished, or the run it points at is gone. Reading that as
          // "message failed to send" sends the user to retry a button that
          // cannot work; the reason is the whole message.
          const key = error instanceof Error ? error.message : "";
          showToast(t(key.includes("continue_unavailable")
            ? "chat.continueUnavailable"
            : "chat.sendFailed"));
        });
    },
    [captureOperation, inputRef],
  );

  /**
   * Fork the conversation through the settled turn that produced this reply,
   * exactly like the desktop's Fork button: the fork point is the preceding
   * *user* entry's persisted identity, never the rendered text. A bubble whose
   * prompt was never persisted says so instead of forking the wrong turn.
   */
  const forkMessage = useCallback(
    (item: TimelineItem) => {
      const { remote, t } = inputRef.current;
      const isCurrent = captureOperation();
      if (item.kind !== "message" || item.role !== "assistant") return;
      const items = remote.timeline.items;
      const index = items.findIndex(entry => entry.id === item.id);
      for (let i = index - 1; i >= 0; i -= 1) {
        const prev = items[i];
        if (prev?.kind === "message" && prev.role === "user") {
          const sourceEntryId = prev.sourceEntryId?.trim();
          if (!sourceEntryId) {
            showToast(t("chat.forkFailed", { message: t("chat.forkNotPersisted") }));
            return;
          }
          void remote.forkConversation(sourceEntryId).catch((error: unknown) => {
            if (!isCurrent()) return;
            const message = error instanceof Error && error.message
              ? error.message
              : t("chat.forkUnknownError");
            showToast(t("chat.forkFailed", { message }));
          });
          return;
        }
      }
    },
    [captureOperation, inputRef],
  );

  return useMemo(() => ({ send, retryMessage, continueMessage, forkMessage }),
    [send, retryMessage, continueMessage, forkMessage]);
}
