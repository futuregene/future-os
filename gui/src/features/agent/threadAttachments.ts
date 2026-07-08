import type { StoredThread } from "../../integrations/storage/threadStore";
import type { MessageAttachment } from "./agentThreadTypes";
import { importAttachmentArtifact, importWorkspaceImage } from "../../integrations/storage/threadStore";
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
 * Copy workspace-mode image originals into the thread's persistent image dir so
 * they survive: workspace conversations don't save attachments into the user's
 * project dir, so the original otherwise lives only in the temp dir (purged by
 * the OS, and deleted after send). Chat-mode attachments are already copied into
 * the temp chat workspace by `importChatAttachments`, so they're skipped here.
 */
export async function importWorkspaceImages(thread: StoredThread, attachments: MessageAttachment[]) {
  if (thread.mode === "chat") {
    return attachments;
  }
  return Promise.all(
    attachments.map(async (attachment) => {
      if (attachment.kind !== "image") {
        return attachment;
      }
      try {
        const path = await importWorkspaceImage({
          name: attachment.name,
          path: attachment.path,
          threadId: thread.id,
        });
        return { ...attachment, path };
      }
      catch {
        // Best-effort: keep the original (temp) path if the copy fails.
        return attachment;
      }
    }),
  );
}

/**
 * Generate a persistent thumbnail for image attachments so the thread can show a
 * small preview without loading the full-size original.
 */
export async function withImageThumbnails(attachments: MessageAttachment[], threadId: string) {
  return Promise.all(
    attachments.map(async (attachment) => {
      if (attachment.kind !== "image") {
        return attachment;
      }
      const thumbnail = await generateImageThumbnail(attachment.path, threadId);
      return thumbnail ? { ...attachment, thumbnail } : attachment;
    }),
  );
}
