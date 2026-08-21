import { useCallback, type Dispatch, type SetStateAction } from "react";
import type { TFunction } from "i18next";
import { File } from "expo-file-system";
import { mimeFor } from "../../remote/files";
import { useRemote } from "../../remote/RemoteContext";
import type { MobileAttachment, TimelineItem } from "../../remote/types";
import { showToast } from "./utils";

type Remote = ReturnType<typeof useRemote>;

export interface SendMessageApi {
  send: () => Promise<void>;
  retryMessage: (item: TimelineItem) => void;
  continueMessage: (item: TimelineItem) => void;
}

export function useSendMessage(
  remote: Remote,
  t: TFunction,
  message: string,
  attachments: MobileAttachment[],
  setMessage: Dispatch<SetStateAction<string>>,
  setAttachments: Dispatch<SetStateAction<MobileAttachment[]>>,
  setTransferProgress: (value: number | null) => void,
): SendMessageApi {
  const send = async () => {
    const value = message.trim();
    if (!value && attachments.length === 0) return;
    const pendingAttachments = attachments;
    setTransferProgress(pendingAttachments.length ? 0 : null);
    try {
      await remote.sendMessage(value, pendingAttachments, (done, total) =>
        setTransferProgress(total > 0 ? done / total : null),
      );
      setMessage("");
      setAttachments([]);
    } catch (error) {
      // M9: sendMessage now throws for busy/streaming/disconnected instead of
      // swallowing the input — always restore the draft so nothing vanishes.
      setMessage(value);
      const key = error instanceof Error ? error.message : "";
      showToast(key === "prompt_too_large" ? t("chat.promptTooLarge") : t("chat.sendFailed"));
    } finally {
      setTransferProgress(null);
    }
  };

  const retryMessage = useCallback(
    (item: TimelineItem) => {
      if (item.kind !== "message" || item.role !== "assistant") return;
      const items = remote.timeline.items;
      const index = items.findIndex(entry => entry.id === item.id);
      for (let i = index - 1; i >= 0; i -= 1) {
        const prev = items[i];
        if (prev?.kind === "message" && prev.role === "user") {
          void (async () => {
            const retryAttachments: MobileAttachment[] = [];
            for (const attachment of prev.attachments ?? []) {
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
            await remote.sendMessage(prev.text, retryAttachments);
          })().catch(() => showToast(t("chat.sendFailed")));
          return;
        }
      }
    },
    [remote, t],
  );

  const continueMessage = useCallback(
    (item: TimelineItem) => {
      if (item.kind !== "message" || item.role !== "assistant" || !item.runId) return;
      void remote
        .continueRun(remote.selectedSessionId, item.runId)
        .catch(() => showToast(t("chat.sendFailed")));
    },
    [remote, t],
  );

  return { send, retryMessage, continueMessage };
}
