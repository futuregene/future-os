import { useState } from "react";
import { useTranslation } from "react-i18next";

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
  if (failed) {
    return (
      <span className="inline-flex max-w-full flex-wrap items-center gap-2 rounded-md border border-dashed border-line-soft bg-surface-subtle px-2 py-1 text-sm text-ink-muted" title={title ?? src}>
        <span className="break-all">{alt || t("image.unavailable")}</span>
        {!linked && <button className="text-accent hover:underline" onClick={() => setFailed(false)} type="button">{t("image.retry")}</button>}
      </span>
    );
  }
  return (
    <span className="my-2 inline-block max-w-full align-top">
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
    </span>
  );
}
