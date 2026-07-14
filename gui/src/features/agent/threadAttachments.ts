import type { MessageAttachment } from "./agentThreadTypes";
import i18n from "../../i18n";
import { deleteTempAttachment, generateImageThumbnail, importEphemeralImage, validateImageAttachment } from "../../integrations/storage/files";

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
  // Phase 1 is read-only: every image must validate before phase 2 moves or
  // deletes any pasted temp original. This keeps a rejected multi-image draft
  // fully retryable instead of leaving some of its paths stale.
  const prepared = await Promise.all(
    attachments.map(async (attachment) => {
      if (attachment.kind !== "image") {
        return { attachment, thumbnail: null };
      }
      // Authoritative readability gate: a pure decode with no side effects. If
      // it can't be decoded the agent would later skip it, so reject the whole
      // send rather than claim the image was attached.
      try {
        await validateImageAttachment(attachment.path);
      }
      catch {
        throw new Error(i18n.t("agent:attachment.imageUnreadable", { name: attachment.name }));
      }
      // The thumbnail is a best-effort nicety for the bubble. A write failure
      // (disk full, permissions) must not block an already-validated image —
      // degrade to no thumbnail instead of rejecting the batch.
      const thumbnail = await generateImageThumbnail({ sourcePath: attachment.path, threadId }).catch(() => null);
      return { attachment, thumbnail };
    }),
  );

  // Phase 2 may persist ephemeral originals now that the whole batch is valid.
  // A missing thumbnail no longer skips persistence — the image is valid and
  // must still get a durable path before its temp original is reclaimed.
  return Promise.all(
    prepared.map(async ({ attachment, thumbnail }) => {
      if (attachment.kind !== "image")
        return attachment;
      let path = attachment.path;
      if (isEphemeralImagePath(path)) {
        try {
          const origin = await importEphemeralImage({ name: attachment.name, path, threadId });
          await deleteTempAttachment(path).catch(() => {});
          path = origin;
        }
        catch {
          // Best-effort: keep the temp path if the durable copy fails.
        }
      }
      return thumbnail ? { ...attachment, path, thumbnail } : { ...attachment, path };
    }),
  );
}
