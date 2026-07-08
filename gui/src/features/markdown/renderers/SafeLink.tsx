import type { ReactNode } from "react";
import { useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { copyText } from "../../../lib/clipboard";
import { LinkContextMenu } from "./LinkContextMenu";
import { useLinkContextMenu } from "./useLinkContextMenu";

/**
 * URL-sanitization layer for markdown links/images. Only protocols on the
 * allowlist survive `safeExternalUrl`; anything else (`javascript:`, `data:`, …)
 * degrades to inert text. External anchors keep `rel="noopener noreferrer"`.
 *
 * Left click follows the anchor (opens in the system browser); right click
 * opens a custom menu (visit / copy link) instead of the webview's native one,
 * matching the local-file link's menu affordance.
 */

export function SafeLink({
  children,
  href,
}: {
  children: ReactNode;
  href: string;
}) {
  const { t } = useTranslation("markdown");
  const anchorRef = useRef<HTMLAnchorElement>(null);
  const menu = useLinkContextMenu();
  const safeHref = safeExternalUrl(href, ["http:", "https:", "mailto:"]);
  if (!safeHref) {
    return <span className="font-medium text-ink-soft" title={href}>{children}</span>;
  }

  return (
    <>
      <a
        className="font-medium text-accent underline-offset-2 hover:underline"
        href={safeHref}
        onContextMenu={menu.open}
        ref={anchorRef}
        rel="noopener noreferrer"
        target="_blank"
        title={href}
      >
        {children}
      </a>
      <LinkContextMenu
        controller={menu}
        items={[
          { label: t("link.visit"), onSelect: () => anchorRef.current?.click() },
          { label: t("link.copyLink"), onSelect: () => void copyText(safeHref) },
        ]}
      />
    </>
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
