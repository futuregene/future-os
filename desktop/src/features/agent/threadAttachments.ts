import type { MessageAttachment } from "@future-os/thread-projection";
import i18n from "../../i18n";
import { deleteTempAttachment, generateImageThumbnail, importEphemeralImage, validateImageAttachment } from "../../integrations/storage/files";

/**
 * New composer entries carry an explicit temporary bit. The path fallback is
 * only for drafts written by older FutureOS versions before that bit existed.
 */
function isEphemeralImage(attachment: MessageAttachment) {
  return attachment.temporary === true
    || (attachment.temporary === undefined && attachment.path.includes("futureos-attachments"));
}

/**
 * Persist image attachments for the thread. Every image gets a cached thumbnail
 * (for the bubble). Pasted/downloaded images — which only ever existed in the
 * temp dir — are additionally copied into `~/.future/app/images/<tid>/origin`
 * and their path rewritten there, so the reference survives after the temp file
 * is cleaned. The source is retained until the agent call succeeds, so an
 * early failure cannot invalidate the attachment path. Local (picked/dragged)
 * images keep their original path and are not copied. Non-image files are
 * untouched.
 */
export async function persistImageAttachments(attachments: MessageAttachment[], threadId: string) {
  // Phase 1 is read-only: every image must validate before phase 2 writes any
  // promoted original. This keeps a rejected multi-image draft
  // internally consistent instead of leaving some of its paths stale.
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
  const persisted = await Promise.all(
    prepared.map(async ({ attachment, thumbnail }) => {
      if (attachment.kind !== "image")
        return { attachment, temporarySource: null };
      let path = attachment.path;
      if (isEphemeralImage(attachment)) {
        try {
          const origin = await importEphemeralImage({ name: attachment.name, path, threadId });
          const temporarySource = path;
          path = origin;
          const promoted = thumbnail
            ? { ...attachment, path, thumbnail, temporary: false }
            : { ...attachment, path, temporary: false };
          return { attachment: promoted, temporarySource };
        }
        catch {
          // Best-effort: keep the temp path if the durable copy fails.
        }
      }
      const current = thumbnail ? { ...attachment, path, thumbnail } : { ...attachment, path };
      return { attachment: current, temporarySource: null };
    }),
  );
  return {
    attachments: persisted.map(item => item.attachment),
    temporarySources: persisted
      .map(item => item.temporarySource)
      .filter((path): path is string => Boolean(path)),
  };
}

/** Delete promoted temp sources only after the agent call has succeeded. */
export async function finalizeTemporaryAttachmentSources(paths: string[]) {
  await Promise.all([...new Set(paths)].map(path => deleteTempAttachment(path).catch(() => {})));
}
