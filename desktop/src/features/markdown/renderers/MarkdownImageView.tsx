import { X } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { useTranslation } from "react-i18next";
import { Overlay } from "../../../components/ui/Overlay";

/**
 * A source-keyed instance prevents an old load/error from poisoning a new URL.
 * Linked images leave activation to their enclosing anchor, never nest buttons.
 */
export function MarkdownImageView(props: { alt: string; src: string; title?: string; linked?: boolean }) {
  return <ImageView key={props.src} {...props} />;
}

function ImageView({ alt, src, title, linked = false }: { alt: string; src: string; title?: string; linked?: boolean }) {
  const { t } = useTranslation("markdown");
  const [failed, setFailed] = useState(false);
  const [expanded, setExpanded] = useState(false);
  const [large, setLarge] = useState(false);
  const [previewOpen, setPreviewOpen] = useState(false);
  if (failed) {
    return (
      <span className="inline-flex max-w-full flex-wrap items-center gap-2 rounded-md border border-dashed border-line-soft bg-surface-subtle px-2 py-1 text-sm text-ink-muted" title={title ?? src}>
        <span className="break-all">{alt || t("image.unavailable")}</span>
        {!linked && <button className="text-accent hover:underline" onClick={() => setFailed(false)} type="button">{t("image.retry")}</button>}
      </span>
    );
  }
  const image = (
    <img
      alt={alt}
      className={`h-auto w-auto max-w-full rounded-md border border-line-soft object-contain ${expanded ? "max-h-none" : "max-h-80"}`}
      decoding="async"
      loading="lazy"
      onError={() => setFailed(true)}
      onLoad={event => setLarge(event.currentTarget.naturalHeight > 320)}
      src={src}
      title={title}
    />
  );
  return (
    <span className="my-2 inline-block max-w-full align-top">
      {linked
        ? image
        : (
            <button
              aria-haspopup="dialog"
              aria-label={t("image.preview", { name: alt || title || t("image.defaultName") })}
              className="block max-w-full cursor-zoom-in rounded-md focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-focus"
              onClick={() => setPreviewOpen(true)}
              type="button"
            >
              {image}
            </button>
          )}
      {!linked && large && (
        <button
          aria-expanded={expanded}
          className="mt-1 block text-xs text-accent hover:underline"
          onClick={() => setExpanded(value => !value)}
          type="button"
        >
          {t(expanded ? "image.collapse" : "image.expand")}
        </button>
      )}
      {!linked && previewOpen && createPortal(
        <ImageLightbox alt={alt} onClose={() => setPreviewOpen(false)} src={src} />,
        document.body,
      )}
    </span>
  );
}

function ImageLightbox({ alt, src, onClose }: { alt: string; src: string; onClose: () => void }) {
  const { t } = useTranslation("markdown");
  const closeRef = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    const previousFocus = document.activeElement;
    closeRef.current?.focus();
    return () => {
      if (previousFocus instanceof HTMLElement && previousFocus.isConnected)
        previousFocus.focus();
    };
  }, []);

  // Portal out of the Markdown tree: streamed blocks use CSS containment and
  // thinking blocks override descendant colors, neither should affect a modal.
  return (
    <div
      aria-label={alt || t("image.defaultName")}
      aria-modal="true"
      onKeyDown={(event) => {
        if (event.key === "Tab") {
          event.preventDefault();
          closeRef.current?.focus();
        }
      }}
      role="dialog"
    >
      <Overlay backdropBlur="strong" onClose={onClose} open>
        <button
          aria-label={t("filePreview.close")}
          className="fixed right-4 top-4 z-10 inline-flex size-9 items-center justify-center rounded-md bg-surface/80 text-ink shadow-panel hover:bg-surface focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-focus"
          onClick={onClose}
          ref={closeRef}
          type="button"
        >
          <X className="size-5" />
        </button>
        <img
          alt={alt}
          className="relative max-h-[calc(100vh-4rem)] max-w-[calc(100vw-4rem)] rounded-md object-contain shadow-panel"
          onError={onClose}
          src={src}
        />
      </Overlay>
    </div>
  );
}
