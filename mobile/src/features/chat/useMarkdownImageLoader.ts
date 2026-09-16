import { useLayoutEffect, useMemo, useRef } from "react";
import type { TFunction } from "i18next";
import { downloadWarning } from "./downloadPolicy";
import { basename } from "@future-os/markdown";
import type { MarkdownImageLoader } from "../../components/MarkdownImage";
import type { useRemote } from "../../remote/RemoteContext";
import { MAX_FILE_BYTES, TransferCancelledError } from "../../remote/files";
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
  // pending" left every image after the first unloaded.
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
      if (cached) return cached.file.uri;
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
      return file.uri;
    };
    // Each caller still gets its own result (and its own rejection).
    return {
      scope,
      cached(_path) {
        // A Markdown path can be overwritten in place. Revalidate its content
        // identity before exposing cached bytes.
        return null;
      },
      load(path, signal) {
        const run = queue.current.then(() => transfer(path, signal));
        queue.current = run.then(() => undefined, () => undefined);
        return run;
      },
    };
  }, [scope, cachedAttachment, prepareAttachment, downloadAttachment, t, queue]);
}
