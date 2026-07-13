import type { MessageAttachment } from "./agentThreadTypes";
import { deleteTempAttachment, generateImageThumbnail, importWorkspaceImage } from "../../integrations/storage/files";

/**
 * A pasted/downloaded image lives in our temp dir (`futureos-attachments`) and
 * has no durable filesystem original; a picked/dragged image has a real path the
 * user owns. We key off the temp marker to decide whether the origin must be
 * persisted. Matches the Rust `delete_temp_attachment` guard's dir.
 */
function isEphemeralImagePath(path: string) {
  return path.includes("futureos-attachments");
}

/**
 * Persist image attachments for the thread. Every image gets a cached thumbnail
 * (for the bubble). Pasted/downloaded images — which only ever existed in the
 * temp dir — are additionally copied into `~/.future/app/images/<tid>/origin`
 * and their path rewritten there, so the reference survives after the temp file
 * is cleaned; the temp copy is then removed. Local (picked/dragged) images keep
 * their original path and are not copied. Non-image files are untouched — they
 * are referenced by their original path and read by the agent on demand.
 */
export async function persistImageAttachments(attachments: MessageAttachment[], threadId: string) {
  return Promise.all(
    attachments.map(async (attachment) => {
      if (attachment.kind !== "image") {
        return attachment;
      }
      let path = attachment.path;
      if (isEphemeralImagePath(path)) {
        try {
          const origin = await importWorkspaceImage({ name: attachment.name, path, threadId });
          await deleteTempAttachment(path).catch(() => {});
          path = origin;
        }
        catch {
          // Best-effort: keep the temp path if the durable copy fails.
        }
      }
      const thumbnail = await generateImageThumbnail({ sourcePath: path, threadId }).catch(() => null);
      return thumbnail ? { ...attachment, path, thumbnail } : { ...attachment, path };
    }),
  );
}
