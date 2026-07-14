import { useEffect, useRef } from "react";
import { useTranslation } from "react-i18next";
import { readTextFilePreview } from "../../integrations/storage/files";
import { useAsyncResource } from "../../lib/useAsyncResource";
import { MarkdownContent } from "../markdown/MarkdownContent";
import { PreviewNotice } from "./PreviewNotice";

/**
 * Renders a local `.md` file with the same renderer the chat uses
 * (`MarkdownContent`), constrained to the conversation-flow width (`max-w-3xl`)
 * with generous vertical padding. Content comes through the text-preview backend
 * command (default 200KB, 1MB cap); a read failure routes to `onError`.
 */
export function MarkdownPreview({
  path,
  onError,
}: {
  path: string;
  onError: () => void;
}) {
  const { t } = useTranslation("markdown");
  const { data: result, error, loading } = useAsyncResource<{ content: string; size: number; truncated: boolean } | null>(
    () => readTextFilePreview({ path }),
    [path],
    null,
  );
  // See ImagePreview: keep onError in a ref so the failure effect doesn't
  // re-fire when callers pass a fresh callback each render.
  const onErrorRef = useRef(onError);
  onErrorRef.current = onError;

  // A read failure routes to `onError` so the overlay falls back to the OS
  // default handler.
  useEffect(() => {
    if (error)
      onErrorRef.current();
  }, [error]);

  const content = loading || error || result == null ? null : result.content;

  if (content == null)
    return <PreviewNotice message={t("filePreview.loading")} />;

  // The surface/rounded frame lives on the scroll container (in FilePreviewOverlay)
  // so its rounded corners stay visible at top and bottom while the text scrolls.
  return (
    <div className="px-8 py-10">
      <MarkdownContent basePath={path} content={content} />
    </div>
  );
}
