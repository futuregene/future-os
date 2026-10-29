import { useLayoutEffect, useMemo, useRef } from "react";
import type { TFunction } from "i18next";
import { downloadWarning } from "./downloadPolicy";
import { basename } from "@future-os/markdown";
import type { MarkdownImageLoader } from "../../components/MarkdownImage";
import type { useRemote } from "../../remote/RemoteContext";
import {
  MAX_FILE_BYTES,
  TransferCancelledError,
  cachedPreviewFile,
  forgetPreviewFile,
  previewAspect,
  rememberPreviewAspect,
  rememberPreviewFile,
} from "../../remote/files";
import type { DownloadInfo } from "../../remote/types";
import { confirmDownload, formatBytes } from "./utils";

type Remote = ReturnType<typeof useRemote>;
const imagePreview = (info: DownloadInfo) => info.previewKind === "image"
  && ["image/png", "image/jpeg", "image/webp", "image/bmp"].includes(info.mimeType)
  && info.size > 0 && info.size <= MAX_FILE_BYTES;

/** Inline images load themselves, in a reply body and in a file preview alike:
 * an image written into the Markdown is part of the document the user opened.
 * The desktop authorization, verified disk cache and encrypted transfer are
 * reused unchanged, and a large transfer on an unknown or cellular network
 * still asks first — the guard is about cost, not about permission to read.
 */
export function useMarkdownImageLoader(remote: Remote, t: TFunction): MarkdownImageLoader {
  const { prepareAttachment, cachedAttachment, downloadAttachment } = remote;
  const scope = JSON.stringify([remote.credentials?.pairId, remote.credentials?.expectedDesktopId, remote.selectedSessionId]);
  const currentScope = useRef(scope);
  // Transfers run one at a time, because a second concurrent one would stack
  // native cellular dialogs. They queue rather than being dropped: a reply can
  // start several inline images at once, and the previous "bail out while one is
  // pending" left every image after the first unloaded. A picture already on
  // this phone never enters the queue — see `load` — so re-entering a
  // conversation draws all of them at once instead of one per round trip.
  const queue = useRef<Promise<unknown>>(Promise.resolve());
  useLayoutEffect(() => {
    currentScope.current = scope;
    return () => { currentScope.current = ""; };
  }, [scope]);
  return useMemo(() => {
    const transfer = async (path: string, signal: AbortSignal): Promise<string | null> => {
      const check = () => {
        if (signal.aborted || currentScope.current !== scope) throw new TransferCancelledError();
      };
      check();
      const attachment = { path, name: basename(path) };
      const info = await prepareAttachment(attachment, "preview", signal);
      check();
      if (!imagePreview(info)) throw new Error("markdown_image_unavailable");
      const cached = cachedAttachment(attachment, "preview");
      if (cached) {
        rememberPreviewFile(scope, path, info);
        return cached.file.uri;
      }
      const warning = await downloadWarning(info.size);
      check();
      if (warning) {
        const accepted = await confirmDownload(t("attachment.downloadTitle"),
          t(warning, { size: formatBytes(info.size) }), t("chat.cancel"), t("attachment.download"));
        check();
        if (!accepted) return null;
      }
      const file = await downloadAttachment(info, undefined, signal);
      check();
      // Only now are the bytes here, so this is the first moment the path can be
      // answered without the desktop: the next open paints from this entry.
      rememberPreviewFile(scope, path, info);
      return file.uri;
    };
    // The content identity a held preview was stored under is re-checked in the
    // background, never on the paint path. The desktop's answer only rewrites
    // the index: the picture on screen stays, and an answer that no longer
    // matches drops the entry so the next open fetches the new picture.
    const revalidate = async (path: string, contentHash: string) => {
      try {
        const info = await prepareAttachment({ path, name: basename(path) }, "preview");
        if (currentScope.current !== scope) return;
        if (info.contentHash === contentHash) rememberPreviewFile(scope, path, info);
        else forgetPreviewFile(scope, path);
      } catch {
        // Unreachable desktop, or a file the desktop no longer serves: the bytes
        // already on screen are still the best answer available.
      }
    };
    // Each caller still gets its own result (and its own rejection).
    return {
      scope,
      cached(path) {
        // Answered from this phone alone. The desktop still gets asked, but
        // afterwards (see `load`), so it can no longer delay the first frame.
        return cachedPreviewFile(scope, path)?.file.uri ?? null;
      },
      load(path, signal) {
        const held = cachedPreviewFile(scope, path);
        if (held) {
          if (signal.aborted || currentScope.current !== scope) return Promise.reject(new TransferCancelledError());
          if (held.stale) void revalidate(path, held.contentHash);
          return Promise.resolve(held.file.uri);
        }
        const run = queue.current.then(() => transfer(path, signal));
        queue.current = run.then(() => undefined, () => undefined);
        return run;
      },
      aspect(path) {
        return previewAspect(scope, path);
      },
      rememberAspect(path, aspect) {
        rememberPreviewAspect(scope, path, aspect);
      },
    };
  }, [scope, cachedAttachment, prepareAttachment, downloadAttachment, t, queue]);
}
