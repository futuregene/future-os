import type { ImageNavigation } from "./ImagePreview";
import type { PreviewKind } from "./previewKind";
import { ArrowLeft, X } from "lucide-react";
import { useCallback, useState } from "react";
import { useTranslation } from "react-i18next";
import { IconButton } from "../../components/ui/IconButton";
import { Overlay } from "../../components/ui/Overlay";
import { emitFutureEvent } from "../../lib/futureEvents";
import { ImagePreview } from "./ImagePreview";
import { JsonPreview } from "./JsonPreview";
import { MarkdownPreview } from "./MarkdownPreview";
import { previewKindForPath } from "./previewKind";
import { TextPreview } from "./TextPreview";

/** One document in the preview stack — the file the overlay opened for, or a link followed from it. */
interface PreviewEntry {
  kind: PreviewKind;
  name: string;
  path: string;
}

/**
 * Fullscreen preview for a local image / markdown / JSON / code-or-text file: a dimmed backdrop
 * (click or Esc dismisses, via `Overlay`) with a close button pinned top-right,
 * and the auto-sized content centered. When a preview can't load (missing,
 * too large, unreadable) it toasts and closes; if `onOpenExternal` is given it
 * also falls back to opening the file with the OS default handler.
 * `unavailableMessage` overrides the default toast (e.g. an attachment whose
 * original is gone, where there's nothing to open externally).
 *
 * A link inside a previewed Markdown document that points at another previewable
 * file opens *in place, one level deeper* (see `PreviewMarkdownContext`): the
 * reader follows a manual's chapters without leaving the app, stepping back out
 * a level at a time with Esc or the back button. Only the top level renders, so
 * each document loads as if it had been opened on its own.
 *
 * `navigation` is image-only: callers that know the folders their file came from
 * (the file tree) pass the previous/next picture so the overlay can step through
 * them in place. It applies to the opened file only, never to a followed link.
 */
export function FilePreviewOverlay({
  path,
  name,
  kind,
  open,
  onClose,
  onOpenExternal,
  unavailableMessage,
  navigation,
}: {
  path: string;
  name: string;
  kind: PreviewKind;
  open: boolean;
  onClose: () => void;
  onOpenExternal?: () => void;
  unavailableMessage?: string;
  navigation?: ImageNavigation;
}) {
  if (!open)
    return null;

  return (
    <PreviewStack
      // Keyed by the opened file: a caller that swaps the file under an open
      // overlay (the file tree's next/previous picture) starts a fresh stack
      // rather than stacking onto the previous document.
      key={`${kind}\u0000${path}`}
      navigation={navigation}
      onClose={onClose}
      onOpenExternal={onOpenExternal}
      root={{ kind, name, path }}
      unavailableMessage={unavailableMessage}
    />
  );
}

function PreviewStack({
  root,
  navigation,
  onClose,
  onOpenExternal,
  unavailableMessage,
}: {
  root: PreviewEntry;
  navigation?: ImageNavigation;
  onClose: () => void;
  onOpenExternal?: () => void;
  unavailableMessage?: string;
}) {
  const { t } = useTranslation("markdown");
  const [stack, setStack] = useState<PreviewEntry[]>([root]);
  const entry = stack[stack.length - 1] ?? root;
  const nested = stack.length > 1;

  // A file that cannot be read: at the root this keeps the old behavior (toast,
  // close, OS handler). One level deeper, popping back to the document that
  // linked here is the useful answer — throwing the reader into an external app
  // would drop them out of the preview they were reading.
  const handleError = useCallback(() => {
    emitFutureEvent("toast", {
      message: unavailableMessage ?? t("filePreview.unavailable", { name: entry.name }),
      tone: "error",
    });
    if (nested) {
      setStack(previous => previous.slice(0, -1));
      return;
    }
    onClose();
    onOpenExternal?.();
  }, [entry.name, nested, onClose, onOpenExternal, t, unavailableMessage]);

  const openFile = useCallback((file: { path: string; name: string }) => {
    const kind = previewKindForPath(file.path);
    if (kind)
      setStack(previous => [...previous, { kind, name: file.name, path: file.path }]);
  }, []);

  const goBack = useCallback(() => {
    setStack(previous => (previous.length > 1 ? previous.slice(0, -1) : previous));
  }, []);

  // Esc (and the backdrop) step out of a followed document only; the close
  // button still leaves the preview outright from any depth.
  const dismiss = useCallback(() => {
    if (nested)
      goBack();
    else
      onClose();
  }, [goBack, nested, onClose]);

  return (
    <Overlay backdropBlur={entry.kind === "image" ? "strong" : "default"} onClose={dismiss} open>
      {nested
        ? (
            <IconButton
              className="fixed left-4 top-4 z-10 bg-surface/80 text-ink shadow-panel hover:bg-surface"
              icon={<ArrowLeft className="size-5" />}
              label={t("filePreview.back")}
              onClick={goBack}
            />
          )
        : null}
      <IconButton
        className="fixed right-4 top-4 z-10 bg-surface/80 text-ink shadow-panel hover:bg-surface"
        icon={<X className="size-5" />}
        label={t("filePreview.close")}
        onClick={onClose}
      />
      {entry.kind === "image"
        ? (
            <div className="relative z-10 flex max-h-full max-w-full items-center justify-center">
              <ImagePreview
                key={entry.path}
                name={entry.name}
                navigation={nested ? undefined : navigation}
                onError={handleError}
                path={entry.path}
              />
            </div>
          )
        : null}
      {entry.kind === "markdown"
        ? (
            <div className="relative z-10 max-h-full w-full max-w-3xl overflow-y-auto rounded-lg bg-surface shadow-panel">
              <MarkdownPreview key={entry.path} onError={handleError} onOpenPreview={openFile} path={entry.path} />
            </div>
          )
        : null}
      {entry.kind === "json"
        ? (
            <div className="relative z-10 w-full max-w-5xl overflow-hidden rounded-lg bg-surface shadow-panel">
              <JsonPreview key={entry.path} onError={handleError} path={entry.path} />
            </div>
          )
        : null}
      {entry.kind === "text"
        ? (
            <div className="relative z-10 w-full max-w-5xl overflow-hidden rounded-lg bg-surface shadow-panel">
              <TextPreview key={entry.path} onError={handleError} path={entry.path} />
            </div>
          )
        : null}
    </Overlay>
  );
}
