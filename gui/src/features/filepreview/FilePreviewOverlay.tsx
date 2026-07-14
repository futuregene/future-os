import type { PreviewKind } from "./previewKind";
import { X } from "lucide-react";
import { useCallback } from "react";
import { useTranslation } from "react-i18next";
import { IconButton } from "../../components/ui/IconButton";
import { Overlay } from "../../components/ui/Overlay";
import { emitFutureEvent } from "../../lib/futureEvents";
import { ImagePreview } from "./ImagePreview";
import { MarkdownPreview } from "./MarkdownPreview";

/**
 * Fullscreen preview for a local image / markdown file: a dimmed backdrop
 * (click or Esc dismisses, via `Overlay`) with a close button pinned top-right,
 * and the auto-sized content centered. When a preview can't load (missing,
 * too large, unreadable) it toasts and closes; if `onOpenExternal` is given it
 * also falls back to opening the file with the OS default handler.
 * `unavailableMessage` overrides the default toast (e.g. an attachment whose
 * original is gone, where there's nothing to open externally).
 */
export function FilePreviewOverlay({
  path,
  name,
  kind,
  open,
  onClose,
  onOpenExternal,
  unavailableMessage,
}: {
  path: string;
  name: string;
  kind: PreviewKind;
  open: boolean;
  onClose: () => void;
  onOpenExternal?: () => void;
  unavailableMessage?: string;
}) {
  const { t } = useTranslation("markdown");

  const handleError = useCallback(() => {
    emitFutureEvent("toast", {
      message: unavailableMessage ?? t("filePreview.unavailable", { name }),
      tone: "error",
    });
    onClose();
    onOpenExternal?.();
  }, [name, onClose, onOpenExternal, t, unavailableMessage]);

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
