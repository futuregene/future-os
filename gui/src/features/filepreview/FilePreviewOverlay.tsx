import type { PreviewKind } from "./previewKind";
import { X } from "lucide-react";
import { lazy, Suspense, useCallback } from "react";
import { useTranslation } from "react-i18next";
import { IconButton } from "../../components/ui/IconButton";
import { Overlay } from "../../components/ui/Overlay";
import { emitFutureEvent } from "../../lib/futureEvents";
import { ImagePreview } from "./ImagePreview";
import { MarkdownPreview } from "./MarkdownPreview";
import { PreviewNotice } from "./PreviewNotice";

// pdf.js is heavy and touches browser-only globals (DOMMatrix) at import time,
// so keep it out of the static graph: only pull it in when a PDF is previewed.
const PdfScrollPreview = lazy(() =>
  import("./PdfScrollPreview").then(module => ({ default: module.PdfScrollPreview })),
);

/**
 * Fullscreen preview for a local image / PDF / markdown file: a dimmed backdrop
 * (click or Esc dismisses, via `Overlay`) with a close button pinned top-right,
 * and the auto-sized content centered. When a preview can't load (missing,
 * too large, unreadable) it toasts and falls back to opening the file with the
 * OS default handler through `onOpenExternal`.
 */
export function FilePreviewOverlay({
  path,
  name,
  kind,
  open,
  onClose,
  onOpenExternal,
}: {
  path: string;
  name: string;
  kind: PreviewKind;
  open: boolean;
  onClose: () => void;
  onOpenExternal: () => void;
}) {
  const { t } = useTranslation("markdown");

  const handleError = useCallback(() => {
    emitFutureEvent("toast", { message: t("filePreview.unavailable", { name }), tone: "error" });
    onClose();
    onOpenExternal();
  }, [name, onClose, onOpenExternal, t]);

  if (!open)
    return null;

  return (
    <Overlay onClose={onClose} open={open}>
      <IconButton
        className="fixed right-4 top-4 z-10 bg-surface/80 text-ink shadow-panel hover:bg-surface"
        icon={<X className="size-5" />}
        label={t("filePreview.close")}
        onClick={onClose}
      />
      {kind === "image"
        ? (
            <div className="relative z-10 flex max-h-full max-w-full items-center justify-center">
              <ImagePreview name={name} onError={handleError} path={path} />
            </div>
          )
        : null}
      {kind === "pdf"
        ? (
            <div className="relative z-10 max-h-full w-full max-w-[900px] overflow-y-auto rounded-lg bg-surface shadow-panel">
              <Suspense fallback={<PreviewNotice message={t("filePreview.loading")} />}>
                <PdfScrollPreview onError={handleError} path={path} />
              </Suspense>
            </div>
          )
        : null}
      {kind === "markdown"
        ? (
            <div className="relative z-10 max-h-full w-full max-w-3xl overflow-y-auto rounded-lg bg-surface shadow-panel">
              <MarkdownPreview onError={handleError} path={path} />
            </div>
          )
        : null}
    </Overlay>
  );
}
