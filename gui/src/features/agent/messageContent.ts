import type { MessageAttachment } from "./agentThreadTypes";

/**
 * The stored envelope for a `mixed` user message: the visible text plus its
 * attachments and the (invisible) inlined attachment context. Persisted as JSON
 * in the message row's `content`; `parseMessageContent` / `stringifyMessageContent`
 * are the only readers/writers of this shape.
 */
interface StoredMixedMessage {
  attachments?: MessageAttachment[];
  inlineContext?: string;
  text?: string;
  type?: string;
}

function isImagePath(path: string) {
  return /\.(?:jpe?g|png|gif|webp|bmp|svg)$/i.test(path);
}

export function parseMessageContent(content: string, contentType?: string) {
  if (contentType !== "mixed") {
    return { attachments: [] as MessageAttachment[], inlineContext: "", text: content };
  }

  try {
    const parsed = JSON.parse(content) as StoredMixedMessage;
    if (parsed.type !== "user_message")
      return { attachments: [] as MessageAttachment[], inlineContext: "", text: content };

    return {
      attachments: Array.isArray(parsed.attachments) ? parsed.attachments : [],
      inlineContext: parsed.inlineContext ?? "",
      text: parsed.text ?? "",
    };
  }
  catch {
    return { attachments: [] as MessageAttachment[], inlineContext: "", text: content };
  }
}

export function stringifyMessageContent(
  text: string,
  attachments: MessageAttachment[],
  inlineContext?: string,
) {
  return JSON.stringify({
    type: "user_message",
    text,
    attachments,
    inlineContext: inlineContext || undefined,
  });
}

export function imageAttachmentPaths(attachments: MessageAttachment[]) {
  return attachments
    .filter(attachment => attachment.kind === "image" || isImagePath(attachment.path))
    .map(attachment => attachment.path);
}
