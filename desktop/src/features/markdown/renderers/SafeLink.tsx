import type { ReactNode } from "react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { openExternalUrl } from "../../../integrations/storage/files";
import { copyText } from "../../../lib/clipboard";
import { usePreviewMarkdown } from "../PreviewMarkdownContext";
import { LinkContextMenu } from "./LinkContextMenu";
import { useLinkContextMenu } from "./useLinkContextMenu";

/**
 * URL-sanitization layer for markdown links/images. Only protocols on the
 * allowlist survive `safeExternalUrl`; anything else (`javascript:`, `data:`, …)
 * degrades to inert text. External anchors keep `rel="noopener noreferrer"`.
 *
 * Left click opens the target in the system default handler via the backend —
 * a plain `target="_blank"` does nothing inside the Tauri webview. In the chat
 * stream a right click opens a custom menu (visit / copy link); in preview mode
 * there is no custom menu (see `PreviewMarkdownContext`).
 */

export function SafeLink({
  children,
  href,
}: {
  children: ReactNode;
  href: string;
}) {
  const { t } = useTranslation("markdown");
  const menu = useLinkContextMenu();
  const preview = usePreviewMarkdown();
  const safeHref = safeExternalUrl(href, ["http:", "https:", "mailto:"]);
  if (!safeHref) {
    return <span className="font-medium text-ink-soft" title={href}>{children}</span>;
  }

  const open = () => void openExternalUrl(safeHref).catch(() => {});

  return (
    <>
      <a
        className="font-medium text-accent underline-offset-2 hover:underline"
        href={safeHref}
        onClick={(event) => {
          event.preventDefault();
          open();
        }}
        onContextMenu={preview ? event => event.preventDefault() : menu.open}
        rel="noopener noreferrer"
        title={href}
      >
        {children}
      </a>
      {preview
        ? null
        : (
            <LinkContextMenu
              controller={menu}
              items={[
                { label: t("link.visit"), onSelect: open },
                { label: t("link.copyLink"), onSelect: () => void copyText(safeHref) },
              ]}
            />
          )}
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
