import type { MessageAttachment } from "./agentThreadTypes";

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
