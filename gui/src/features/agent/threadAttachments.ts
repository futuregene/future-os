import type { StoredThread } from "../../integrations/storage/threadStore";
import type { MessageAttachment } from "./agentThreadTypes";
import { importAttachmentArtifact } from "../../integrations/storage/threadStore";
import { generateImageThumbnail } from "./attachments";

/**
 * Copy a chat thread's pasted attachments into the artifact store so they get a
 * stable path (chat temp workspaces are ephemeral). Workspace threads keep their
 * original paths. Returns the attachments with `artifactId`/`path` filled in.
 */
export async function importChatAttachments(thread: StoredThread, attachments: MessageAttachment[]) {
  if (thread.mode !== "chat") {
    return attachments;
  }

  return Promise.all(
    attachments.map(async (attachment) => {
      const artifact = await importAttachmentArtifact({
        path: attachment.path,
        threadId: thread.id,
      });

      return {
        ...attachment,
        artifactId: artifact.id,
        path: artifact.path ?? attachment.path,
      };
    }),
  );
}

/**
 * Generate a cached thumbnail for image attachments so the thread can show a
 * small preview without loading the full-size original.
 */
export async function withImageThumbnails(attachments: MessageAttachment[]) {
  return Promise.all(
    attachments.map(async (attachment) => {
      if (attachment.kind !== "image") {
        return attachment;
      }
      const thumbnail = await generateImageThumbnail(attachment.path);
      return thumbnail ? { ...attachment, thumbnail } : attachment;
    }),
  );
}
