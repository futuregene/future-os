import type { MessageAttachment } from "@future-os/thread-projection";
import i18n from "../../i18n";
import { deleteTempAttachment, generateImageThumbnail, importEphemeralAttachment, validateImageAttachment } from "../../integrations/storage/files";

/**
 * New composer entries carry an explicit temporary bit. The path fallback is
 * only for drafts written by older FutureOS versions before that bit existed.
 */
function isEphemeralAttachment(attachment: MessageAttachment) {
  return attachment.temporary === true
    || (attachment.temporary === undefined && attachment.path.includes("futureos-attachments"));
}

export interface UnpersistedAttachment {
  /** The name the chip shows, so the warning can name what did not stick. */
  name: string;
  /** The temp path the message still holds. */
  path: string;
}

/**
 * Persist attachments that FutureOS copied from the clipboard. Every image gets a cached thumbnail
 * (for the bubble). Pasted/downloaded images — which only ever existed in the
 * temp dir — are additionally copied into `~/.future/app/images/<tid>/origin`
 * and their path rewritten there, so the reference survives after the temp file
 * is cleaned. The source is retained until the agent call succeeds, so an
 * early failure cannot invalidate the attachment path. Local (picked/dragged)
 * files keep their original path and are not copied; an ephemeral one (a paste,
 * an image or not) is copied whatever its kind, because the temp dir it came
 * from is not somewhere a message may point at for long.
 *
 * A durable copy that fails is returned in `unpersisted` rather than swallowed:
 * the send still proceeds — the file is readable now and the temp original is
 * kept — but the caller has to say so *then*, because from then on the bubble
 * holds a path the OS will reclaim, and a reader who learns that later sees
 * "the file was moved or deleted" about a file they never touched.
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
      let path = attachment.path;
      if (isEphemeralAttachment(attachment)) {
        try {
          const origin = await importEphemeralAttachment({ name: attachment.name, path, threadId });
          const temporarySource = path;
          path = origin;
          const promoted = attachment.kind === "image" && thumbnail
            ? { ...attachment, path, thumbnail, temporary: false }
            : { ...attachment, path, temporary: false };
          return { attachment: promoted, temporarySource, unpersisted: null };
        }
        catch {
          // Degrade rather than block the send: the file is readable right now,
          // so this turn still reaches the model with its content. What is lost
          // is the copy the bubble will point at later — reported to the caller,
          // never swallowed, and the temp source is kept (nothing finalizes a
          // path this never promoted).
          const current = attachment.kind === "image" && thumbnail ? { ...attachment, thumbnail } : attachment;
          return {
            attachment: current,
            temporarySource: null,
            unpersisted: { name: attachment.name, path: attachment.path },
          };
        }
      }
      const current = attachment.kind === "image" && thumbnail ? { ...attachment, path, thumbnail } : { ...attachment, path };
      return { attachment: current, temporarySource: null, unpersisted: null };
    }),
  );
  return {
    attachments: persisted.map(item => item.attachment),
    temporarySources: persisted
      .map(item => item.temporarySource)
      .filter((path): path is string => Boolean(path)),
    /** Ephemeral attachments still pointing at the temp dir, and why. */
    unpersisted: persisted
      .map(item => item.unpersisted)
      .filter((item): item is UnpersistedAttachment => item !== null),
  };
}

/** Delete promoted temp sources only after the agent call has succeeded. */
export async function finalizeTemporaryAttachmentSources(paths: string[]) {
  await Promise.all([...new Set(paths)].map(path => deleteTempAttachment(path).catch(() => {})));
}
