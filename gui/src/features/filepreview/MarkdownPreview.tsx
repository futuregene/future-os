import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { readTextFilePreview } from "../../integrations/storage/files";
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
  const [content, setContent] = useState<string | null>(null);
  // See ImagePreview: keep onError in a ref so the effect depends only on `path`.
  const onErrorRef = useRef(onError);
  onErrorRef.current = onError;

  useEffect(() => {
    let cancelled = false;
    setContent(null);
    readTextFilePreview({ path })
      .then((result) => {
        if (!cancelled)
          setContent(result.content);
      })
      .catch(() => {
        if (!cancelled)
          onErrorRef.current();
      });
    return () => {
      cancelled = true;
    };
  }, [path]);

  if (content == null)
    return <PreviewNotice message={t("filePreview.loading")} />;

  return (
    <div className="my-2 rounded-lg bg-surface px-8 py-10 shadow-panel">
      <MarkdownContent content={content} />
    </div>
  );
}
