import type { MessageAttachment } from "./agentThreadTypes";

/**
 * The stored envelope for a `mixed` user message: the visible text plus its
 * attachments. Persisted as JSON in the message row's `content`;
 * `parseMessageContent` / `stringifyMessageContent` are the only readers/writers.
 */
interface StoredMixedMessage {
  attachments?: MessageAttachment[];
  text?: string;
  type?: string;
}

export function parseMessageContent(content: string, contentType?: string) {
  if (contentType !== "mixed") {
    return { attachments: [] as MessageAttachment[], text: content };
  }

  try {
    const parsed = JSON.parse(content) as StoredMixedMessage;
    if (parsed.type !== "user_message")
      return { attachments: [] as MessageAttachment[], text: content };

    return {
      attachments: Array.isArray(parsed.attachments) ? parsed.attachments : [],
      text: parsed.text ?? "",
    };
  }
  catch {
    return { attachments: [] as MessageAttachment[], text: content };
  }
}

export function stringifyMessageContent(text: string, attachments: MessageAttachment[]) {
  return JSON.stringify({ type: "user_message", text, attachments });
}

/** The wire shape sent to the agent: original path + kind + display name. */
export interface AttachmentInput {
  path: string;
  kind: "image" | "file";
  name: string;
  /** Cached-thumbnail path (images only); persisted in the entry meta for reload. */
  thumbnail?: string;
}

export function attachmentInputs(attachments: MessageAttachment[]): AttachmentInput[] {
  return attachments.map(attachment => ({
    path: attachment.path,
    kind: attachment.kind === "image" ? "image" : "file",
    name: attachment.name,
    ...(attachment.thumbnail ? { thumbnail: attachment.thumbnail } : {}),
  }));
}
