import type { GalleryImage } from "../imageGallery";
import { ChevronLeft, ChevronRight, X } from "lucide-react";
import { useCallback, useEffect, useId, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { useTranslation } from "react-i18next";
import { Overlay } from "../../../components/ui/Overlay";
import { useImageGallery } from "../imageGallery";

/**
 * A source-keyed instance prevents an old load/error from poisoning a new URL.
 * Linked images leave activation to their enclosing anchor, never nest buttons.
 */
export function MarkdownImageView(props: { alt: string; src: string; title?: string; linked?: boolean; width?: number }) {
  return <ImageView key={props.src} {...props} />;
}

/** What the lightbox is showing: the drawing's pictures and where it started. */
interface OpenPreview {
  images: GalleryImage[];
  index: number;
}

function ImageView({ alt, src, title, linked = false, width }: { alt: string; src: string; title?: string; linked?: boolean; width?: number }) {
  const { t } = useTranslation("markdown");
  const [failed, setFailed] = useState(false);
  const [preview, setPreview] = useState<OpenPreview | null>(null);
  const gallery = useImageGallery();
  const galleryId = useId();

  // Join this document's gallery. Registration happens per mounted picture, so
  // the Map's insertion order is the order the reply drew them — the same order
  // the reader scrolls through, and the order the arrows step in.
  useEffect(() => {
    if (!gallery || linked || failed)
      return;
    gallery.add({ alt, id: galleryId, src, title });
    return () => gallery.remove(galleryId);
  }, [gallery, galleryId, alt, src, title, linked, failed]);

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
      className="h-auto w-auto max-h-80 max-w-full rounded-md border border-line-soft object-contain"
      decoding="async"
      loading="lazy"
      onError={() => setFailed(true)}
      src={src}
      style={width ? { maxWidth: width } : undefined}
      title={title}
    />
  );
  // A drawing's pictures belong together: open with the document's list so the
  // lightbox can step through them without going back to the transcript. A
  // surface with no gallery (a bare `MarkdownImageView` in a test or a host
  // without `MarkdownContent`) still opens the single picture it was given.
  const openPreview = () => {
    const images = gallery?.snapshot() ?? [];
    const at = images.findIndex(candidate => candidate.id === galleryId);
    setPreview(at >= 0 ? { images, index: at } : { images: [{ alt, id: galleryId, src, title }], index: 0 });
  };
  return (
    <span className="my-2 block max-w-full" style={width ? { display: "flex", justifyContent: "center" } : undefined}>
      {linked
        ? image
        : (
            <button
              aria-haspopup="dialog"
              aria-label={t("image.preview", { name: alt || title || t("image.defaultName") })}
              className="block max-w-full cursor-zoom-in rounded-md focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-focus"
              onClick={openPreview}
              type="button"
            >
              {image}
            </button>
          )}
      {!linked && preview && createPortal(
        <ImageLightbox images={preview.images} index={preview.index} onClose={() => setPreview(null)} />,
        document.body,
      )}
    </span>
  );
}

function ImageLightbox({ images, index: initialIndex, onClose }: OpenPreview & { onClose: () => void }) {
  const { t } = useTranslation("markdown");
  const closeRef = useRef<HTMLButtonElement>(null);
  const [index, setIndex] = useState(initialIndex);
  // One step per deliberate horizontal swipe: a trackpad delivers a trail of
  // wheel events, and without this a single flick would run through the set.
  const lastStepRef = useRef(0);
  const count = images.length;
  const current = images[index] ?? images[0]!;
  const step = useCallback((delta: number) => {
    if (count < 2)
      return;
    // Both ends wrap: the sequence is a ring, not a line, so the arrow never
    // becomes a dead control that only says "you are at the end".
    setIndex(previous => (previous + delta + count) % count);
  }, [count]);

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
      aria-label={current.alt || current.title || t("image.defaultName")}
      aria-modal="true"
      onKeyDown={(event) => {
        if (event.key === "Tab") {
          event.preventDefault();
          closeRef.current?.focus();
          return;
        }
        if (event.key === "ArrowLeft") {
          event.preventDefault();
          step(-1);
        }
        else if (event.key === "ArrowRight") {
          event.preventDefault();
          step(1);
        }
      }}
      onWheel={(event) => {
        // The desktop's "swipe": a trackpad's horizontal gesture (or a
        // horizontal mouse wheel) is a sideways scroll, so page it. Vertical
        // scrolling stays untouched.
        if (count < 2 || Math.abs(event.deltaX) <= Math.abs(event.deltaY) || Math.abs(event.deltaX) < 30)
          return;
        const now = Date.now();
        if (now - lastStepRef.current < 320)
          return;
        lastStepRef.current = now;
        step(event.deltaX > 0 ? 1 : -1);
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
        {count > 1
          ? (
              <>
                <NavButton label={t("filePreview.prevImage")} onClick={() => step(-1)} side="left">
                  <ChevronLeft className="size-6" />
                </NavButton>
                <NavButton label={t("filePreview.nextImage")} onClick={() => step(1)} side="right">
                  <ChevronRight className="size-6" />
                </NavButton>
                <span className="fixed bottom-6 left-1/2 z-10 -translate-x-1/2 rounded-full bg-surface/80 px-3 py-1 text-xs tabular-nums text-ink-soft shadow-panel">
                  {index + 1}
                  {" / "}
                  {count}
                </span>
              </>
            )
          : null}
        <img
          alt={current.alt}
          className="relative max-h-[calc(100vh-4rem)] max-w-[calc(100vw-4rem)] rounded-md object-contain shadow-panel"
          onError={onClose}
          src={current.src}
        />
      </Overlay>
    </div>
  );
}

function NavButton({ children, label, onClick, side }: { children: React.ReactNode; label: string; onClick: () => void; side: "left" | "right" }) {
  return (
    <button
      aria-label={label}
      className={`fixed ${side === "left" ? "left-4" : "right-4"} top-1/2 z-10 inline-flex size-10 -translate-y-1/2 items-center justify-center rounded-full bg-surface/80 text-ink shadow-panel hover:bg-surface focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-focus`}
      onClick={onClick}
      title={label}
      type="button"
    >
      {children}
    </button>
  );
}
