import { useCallback, useMemo, type Dispatch, type SetStateAction } from "react";
import type { TFunction } from "i18next";
import { File } from "expo-file-system";
import { classifyNatsError } from "@future-os/remote-client/natsErrors";
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

type FailureText = { key: string; params?: Record<string, unknown> };

/**
 * Say what actually happened instead of one catch-all sentence.
 *
 * "Message failed to send" was covering four different things — the desktop
 * refusing because a run is still in flight, this device having no connection,
 * a reply that can no longer be continued, and a genuine send fault — so the
 * only thing a reader could do with it was guess. Everything the clients can
 * recognize gets its own line now; the unknown remainder keeps the old one.
 */
function sendFailureText(error: unknown, holder: string | undefined): FailureText {
  const code = error instanceof Error ? error.message : "";
  // `superseded` is the desktop refusing because another device holds it. The
  // composer's own notice says so while the pointer already reads standby; this
  // branch is the race — control moved a moment ago.
  if (code === "superseded")
    return holder
      ? { key: "chat.standbyCannotSend", params: { name: holder } }
      : { key: "chat.standbyCannotSendUnknown" };
  if (code.includes("session_busy")) return { key: "chat.sessionBusy" };
  if (code.includes("continue_unavailable")) return { key: "chat.continueUnavailable" };
  if (code.includes("session_not_on_desktop")) return { key: "chat.sessionNotOnDesktop" };
  if (code === "send_compacting") return { key: "chat.compacting" };
  if (code === "prompt_too_large") return { key: "chat.promptTooLarge" };
  if (code === "send_busy") return { key: "chat.sendInFlight" };
  // Nothing leaves this device without the pairing: that is a connection
  // problem, not something wrong with what the user typed — and a request that
  // timed out or lost its socket is the same story.
  if (code === "not_connected" || code === "pairing_changed"
    || code === "remote_access_changed") return { key: "chat.sendOffline" };
  if (classifyNatsError(error)) return { key: "chat.sendOffline" };
  return { key: "chat.sendFailed" };
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
      const failure = sendFailureText(error, remote.presence?.holder?.name?.trim());
      showToast(t(failure.key, failure.params));
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
          })().catch((error: unknown) => {
            if (!isCurrent()) return;
            const failure = sendFailureText(error, undefined);
            showToast(t(failure.key, failure.params));
          });
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
          const failure = sendFailureText(error, remote.presence?.holder?.name?.trim());
          showToast(t(failure.key, failure.params));
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
