import type { MessageAttachment } from "./types";

export const MAX_ATTACHMENTS_PER_TURN = 4;

export const IMAGE_EXTENSIONS = ["jpg", "jpeg", "png", "gif", "webp", "bmp", "svg"] as const;

const IMAGE_MIME_EXTENSIONS: Record<string, string> = {
  "image/bmp": "bmp",
  "image/gif": "gif",
  "image/jpeg": "jpg",
  "image/png": "png",
  "image/svg+xml": "svg",
  "image/webp": "webp",
};

export function imageExtensionFromMime(mime: string) {
  return IMAGE_MIME_EXTENSIONS[mime.toLowerCase()] ?? null;
}

interface StoredMixedMessage {
  attachments?: MessageAttachment[];
  text?: string;
  type?: string;
}

export function fileNameFromPath(path: string) {
  const segments = path.split(/[\\/]/).filter(Boolean);
  return segments.length > 0 ? segments[segments.length - 1] : path;
}

export function parseMessageContent(content: string, contentType?: string) {
  if (contentType !== "mixed") {
    return { attachments: [], text: content };
  }

  try {
    const parsed = JSON.parse(content) as StoredMixedMessage;
    if (parsed.type !== "user_message")
      return { attachments: [], text: content };

    return {
      attachments: Array.isArray(parsed.attachments) ? parsed.attachments : [],
      text: parsed.text ?? "",
    };
  }
  catch {
    return { attachments: [], text: content };
  }
}

export function stringifyMessageContent(text: string, attachments: MessageAttachment[]) {
  return JSON.stringify({
    type: "user_message",
    text,
    attachments,
  });
}

export function buildPromptWithAttachments(text: string, attachments: MessageAttachment[]) {
  if (attachments.length === 0)
    return text;

  const body = text.trim() || "请参考附件。";
  const attachmentLines = attachments
    .map((attachment, index) => `${index + 1}. ${attachment.name}: ${attachment.path}`)
    .join("\n");

  return `${body}\n\nAttached files for context:\n${attachmentLines}`;
}

export function imageAttachmentPaths(attachments: MessageAttachment[]) {
  return attachments
    .filter(attachment => isImagePath(attachment.path))
    .map(attachment => attachment.path);
}

export function isImagePath(path: string) {
  return /\.(?:jpe?g|png|gif|webp|bmp|svg)$/i.test(path);
}
