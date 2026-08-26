import type { UIEvent } from "react";
import {
  formatJsonForPreview,
  MAX_JSON_RICH_PREVIEW_BYTES,
  rawJsonLines,
  tokenizeJsonLine,
} from "@future-os/json-preview";
import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { readTextFilePreview } from "../../integrations/storage/files";
import { useAsyncResource } from "../../lib/useAsyncResource";
import { PreviewNotice } from "./PreviewNotice";
import { usePreviewLoadingGate } from "./usePreviewLoadingGate";

const ROW_HEIGHT = 22;
const OVERSCAN_ROWS = 24;

export function JsonPreview({
  path,
  onError,
}: {
  path: string;
  onError: () => void;
}) {
  const { t } = useTranslation("markdown");
  const { data: result, error, loading } = useAsyncResource(
    () => readTextFilePreview({ maxBytes: MAX_JSON_RICH_PREVIEW_BYTES, path }),
    [path],
    null,
  );
  const onErrorRef = useRef(onError);
  onErrorRef.current = onError;
  const gate = usePreviewLoadingGate(loading);

  useEffect(() => {
    if (error && gate.showContent)
      onErrorRef.current();
  }, [error, gate.showContent]);

  const prepared = useMemo(() => {
    if (!result)
      return null;
    const sourceLimited = result.truncated || result.size >= MAX_JSON_RICH_PREVIEW_BYTES;
    let validationError: string | null = null;
    if (!sourceLimited && result.validUtf8) {
      try {
        JSON.parse(result.content);
      }
      catch (parseError) {
        validationError = parseError instanceof Error ? parseError.message : String(parseError);
      }
    }
    const formatted = sourceLimited || !result.validUtf8 || validationError
      ? rawJsonLines(result.content)
      : formatJsonForPreview(result.content);
    return { formatted, sourceLimited, validationError, validUtf8: result.validUtf8 };
  }, [result]);

  if (gate.showLoading)
    return <PreviewNotice message={t("filePreview.loading")} />;

  if (!gate.showContent || error || !prepared)
    return null;

  return (
    <div className="flex h-[min(80vh,48rem)] min-h-0 flex-col bg-surface text-sm text-ink">
      {prepared.sourceLimited || !prepared.validUtf8 || prepared.validationError || prepared.formatted.limited
        ? (
            <div className="space-y-1 border-b border-line-soft bg-surface-subtle px-4 py-3 text-sm">
              {prepared.sourceLimited
                ? <p className="text-warning">{t("jsonPreview.truncated")}</p>
                : null}
              {!prepared.validUtf8
                ? <p className="text-danger">{t("jsonPreview.invalidEncoding")}</p>
                : null}
              {prepared.validationError
                ? <p className="text-danger">{t("jsonPreview.invalid", { detail: prepared.validationError })}</p>
                : null}
              {prepared.formatted.limited
                ? <p className="text-warning">{t("jsonPreview.tooComplex")}</p>
                : null}
            </div>
          )
        : null}
      <VirtualJsonLines lines={prepared.formatted.lines} />
    </div>
  );
}

function VirtualJsonLines({ lines }: { lines: string[] }) {
  const [viewport, setViewport] = useState({ height: 640, scrollTop: 0 });
  const start = Math.max(0, Math.floor(viewport.scrollTop / ROW_HEIGHT) - OVERSCAN_ROWS);
  const visibleRows = Math.ceil(viewport.height / ROW_HEIGHT) + OVERSCAN_ROWS * 2;
  const end = Math.min(lines.length, start + visibleRows);

  function handleScroll(event: UIEvent<HTMLDivElement>) {
    const target = event.currentTarget;
    setViewport({ height: target.clientHeight, scrollTop: target.scrollTop });
  }

  return (
    <div className="min-h-0 flex-1 overflow-auto" onScroll={handleScroll}>
      <div className="relative min-w-max" style={{ height: lines.length * ROW_HEIGHT }}>
        {lines.slice(start, end).map((line, offset) => {
          const index = start + offset;
          return (
            <div
              className="absolute left-0 flex min-w-full font-mono text-[13px] leading-[22px]"
              key={index}
              style={{ top: index * ROW_HEIGHT }}
            >
              <span className="sticky left-0 w-14 shrink-0 select-none border-r border-line-soft bg-surface pr-3 text-right text-ink-muted">
                {index + 1}
              </span>
              <code className="whitespace-pre px-3">
                {tokenizeJsonLine(line).map((token, tokenIndex) => (
                  // Tokens have no source identity and duplicates are valid JSON.
                  // eslint-disable-next-line react/no-array-index-key
                  <span className={jsonTokenClass(token.kind)} key={tokenIndex}>{token.text}</span>
                ))}
              </code>
            </div>
          );
        })}
      </div>
    </div>
  );
}

function jsonTokenClass(kind: ReturnType<typeof tokenizeJsonLine>[number]["kind"]) {
  switch (kind) {
    case "key":
      return "text-accent";
    case "string":
      return "text-success";
    case "number":
      return "text-warning";
    case "literal":
      return "text-info";
    default:
      return "text-ink";
  }
}
