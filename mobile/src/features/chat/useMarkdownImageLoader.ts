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

/** User-initiated only: reuse the existing desktop authorization, verified
 * disk cache and encrypted transfer; do not turn Markdown into an auto-reader.
 */
export function useMarkdownImageLoader(remote: Remote, t: TFunction): MarkdownImageLoader {
  const { prepareAttachment, cachedAttachment, downloadAttachment } = remote;
  const scope = JSON.stringify([remote.credentials?.pairId, remote.credentials?.expectedDesktopId, remote.selectedSessionId]);
  const currentScope = useRef(scope);
  const pending = useRef<symbol | null>(null);
  useLayoutEffect(() => {
    currentScope.current = scope;
    pending.current = null;
    return () => { currentScope.current = ""; };
  }, [scope]);
  return useMemo(() => {
    return {
      scope,
      cached(path) {
        if (currentScope.current !== scope) return null;
        const cached = cachedAttachment({ path, name: basename(path) }, "preview");
        return cached && imagePreview(cached.info) ? cached.file.uri : null;
      },
      async load(path, signal) {
        const check = () => {
          if (signal.aborted || currentScope.current !== scope) throw new TransferCancelledError();
        };
        check();
        // Avoid stacking several native cellular-confirmation dialogs.
        if (pending.current) return null;
        const request = Symbol();
        pending.current = request;
        try {
          const attachment = { path, name: basename(path) };
          let cached = cachedAttachment(attachment, "preview");
          const info = cached?.info ?? await prepareAttachment(attachment, "preview", signal);
          check();
          if (!imagePreview(info)) throw new Error("markdown_image_unavailable");
          cached ??= cachedAttachment(attachment, "preview");
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
        } finally { if (pending.current === request) pending.current = null; }
      },
    };
  }, [scope, cachedAttachment, prepareAttachment, downloadAttachment, t]);
}
