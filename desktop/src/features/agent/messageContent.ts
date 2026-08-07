import type { AttachmentInput } from "../../integrations/agent/agentClient";
import type { MessageAttachment } from "./agentThreadTypes";

export function stringifyMessageContent(text: string, attachments: MessageAttachment[]) {
  return JSON.stringify({ type: "user_message", text, attachments });
}

export function attachmentInputs(attachments: MessageAttachment[]): AttachmentInput[] {
  return attachments.map(attachment => ({
    path: attachment.path,
    kind: attachment.kind === "image" ? "image" : "file",
    name: attachment.name,
    ...(attachment.thumbnail ? { thumbnail: attachment.thumbnail } : {}),
  }));
}
