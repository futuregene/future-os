import type { ReactNode } from "react";
import { useState } from "react";
import { useTranslation } from "react-i18next";

/**
 * URL-sanitization layer for markdown links/images. Only protocols on the
 * allowlist survive `safeExternalUrl`; anything else (`javascript:`, `data:`, …)
 * degrades to inert text. External anchors keep `rel="noopener noreferrer"`.
 */

export function SafeLink({
  children,
  href,
}: {
  children: ReactNode;
  href: string;
}) {
  const safeHref = safeExternalUrl(href, ["http:", "https:", "mailto:"]);
  if (!safeHref) {
    return <span className="font-medium text-ink-soft" title={href}>{children}</span>;
  }

  return (
    <a
      className="font-medium text-accent underline-offset-2 hover:underline"
      href={safeHref}
      rel="noopener noreferrer"
      target="_blank"
      title={href}
    >
      {children}
    </a>
  );
}

export function SafeImage({
  alt,
  src,
  title,
}: {
  alt: string;
  src: string;
  title?: string;
}) {
  const { t } = useTranslation("markdown");
  const [failed, setFailed] = useState(false);
  const safeSrc = safeExternalUrl(src, ["http:", "https:"]);
  if (!safeSrc || failed) {
    return (
      <span
        className="inline-flex max-w-full items-center rounded-md border border-dashed border-line-soft bg-surface-subtle px-2 py-1 text-sm text-ink-muted"
        title={src}
      >
        {alt || t("image.unavailable")}
      </span>
    );
  }

  return (
    <img
      alt={alt}
      className="my-2 max-h-80 max-w-full rounded-md border border-line-soft object-contain"
      onError={() => setFailed(true)}
      src={safeSrc}
      title={title}
    />
  );
}

function safeExternalUrl(value: string, allowedProtocols: string[]) {
  try {
    const url = new URL(value);
    return allowedProtocols.includes(url.protocol) ? value : null;
  }
  catch {
    return null;
  }
}
