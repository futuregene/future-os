import { useEffect, useMemo, useRef } from "react";
import { useTranslation } from "react-i18next";
import { readTextFilePreview } from "../../integrations/storage/files";
import { useAsyncResource } from "../../lib/useAsyncResource";
import { HighlightedCode } from "../markdown/HighlightedCode";
import { MAX_HIGHLIGHTABLE_CODE_LENGTH, useCodeHighlighter } from "../markdown/useCodeHighlighter";
import { codeLanguageForPath } from "./previewKind";
import { PreviewNotice } from "./PreviewNotice";
import { usePreviewLoadingGate } from "./usePreviewLoadingGate";

interface TextPreviewResult {
  content: string;
  size: number;
  truncated: boolean;
  validUtf8: boolean;
}

/**
 * Monospace reader for code / config text files (`.py`, `.rs`, `.go`, …) with
 * syntax colors. Content comes through the same backend command as
 * `MarkdownPreview` (default 200KB, 1MB cap): the first chunk is shown and a
 * notice marks it truncated.
 *
 * A read failure — unreadable, or bytes that aren't UTF-8 text (a `.c` that is
 * really a binary) — routes to `onError`, so the overlay falls back to the OS
 * default handler instead of showing replacement characters.
 *
 * Highlighting is bounded and optional at every step: no grammar for the
 * extension, a source past {@link MAX_HIGHLIGHTABLE_CODE_LENGTH}, a grammar
 * still loading, or a tokenizer failure all render the untouched source in the
 * same monospace, so the reader never waits on the highlighter and never sees
 * mangled text.
 */
export function TextPreview({ path, onError }: { path: string; onError: () => void }) {
  const { t } = useTranslation("markdown");
  const { data: result, error, loading } = useAsyncResource<TextPreviewResult | null>(
    () => readTextFilePreview({ path }),
    [path],
    null,
  );
  // See ImagePreview: keep onError in a ref so the failure effect doesn't
  // re-fire when callers pass a fresh callback each render.
  const onErrorRef = useRef(onError);
  onErrorRef.current = onError;
  const gate = usePreviewLoadingGate(loading);
  const binary = result !== null && !result.validUtf8;
  const failed = Boolean(error) || binary;

  const language = useMemo(() => codeLanguageForPath(path), [path]);
  const { highlight } = useCodeHighlighter();
  const content = result?.content;
  const highlighted = useMemo(() => {
    if (!language || content === undefined || content.length > MAX_HIGHLIGHTABLE_CODE_LENGTH)
      return null;
    return highlight(content, language);
  }, [content, highlight, language]);

  useEffect(() => {
    if (failed && gate.showContent)
      onErrorRef.current();
  }, [failed, gate.showContent]);

  if (gate.showLoading)
    return <PreviewNotice message={t("filePreview.loading")} />;

  if (!gate.showContent || result == null || failed)
    return null;

  return (
    <div className="max-h-[calc(100vh-6rem)] overflow-auto">
      {result.truncated
        ? (
            <div className="sticky top-0 border-b border-line-soft bg-warning-soft px-4 py-2 text-xs text-warning">
              {t("filePreview.truncated")}
            </div>
          )
        : null}
      <pre
        className="min-w-full px-4 py-3 text-[11px] leading-4 whitespace-pre-wrap text-ink-soft"
        style={highlighted ? { color: highlighted.fgColor } : undefined}
      >
        <code>
          {highlighted
            ? <HighlightedCode code={result.content} highlighted={highlighted} />
            : result.content}
        </code>
      </pre>
    </div>
  );
}
