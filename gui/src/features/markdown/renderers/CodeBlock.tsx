import { useMemo } from "react";
import { useTranslation } from "react-i18next";
import { CopyButton } from "../../../components/ui/CopyButton";
import { useCopyState } from "../../../components/ui/useCopyState";
import { useCodeHighlighter } from "../useCodeHighlighter";

export function CodeBlock({
  code,
  language,
}: {
  code: string;
  language?: string;
}) {
  const { t } = useTranslation("markdown");
  const { copiedKey, copy } = useCopyState();
  const { highlight, isLoaded } = useCodeHighlighter();
  const highlighted = useMemo(() => highlight(code, language), [highlight, code, language]);

  // Fallback to plain text if highlighter not loaded or language not supported
  if (!isLoaded || !highlighted) {
    return (
      <div className="relative">
        <CopyButton
          copied={copiedKey !== null}
          label={t("codeBlock.copyCode")}
          onCopy={() => void copy(code)}
          variant="floating"
        />
        <pre className="overflow-auto rounded-lg border border-line-soft bg-surface-subtle p-3 pr-11 text-xs leading-5 text-ink">
          {language ? <div className="mb-2 text-[11px] text-ink-muted">{language}</div> : null}
          <code>{code}</code>
        </pre>
      </div>
    );
  }

  return (
    <div className="relative">
      <CopyButton
        className="z-10"
        copied={copiedKey !== null}
        label={t("codeBlock.copyCode")}
        onCopy={() => void copy(code)}
        variant="floating"
      />
      <pre
        className="overflow-auto rounded-lg border border-line-soft p-3 pr-11 text-xs leading-5"
        style={{ backgroundColor: highlighted.bgColor, color: highlighted.fgColor }}
      >
        {language ? <div className="mb-2 text-[11px] opacity-60">{language}</div> : null}
        <code>
          {highlighted.lines.map((line, lineIndex) => (
            // eslint-disable-next-line react/no-array-index-key -- static positional render of highlighted code; lines never reorder
            <div key={lineIndex} className="flex">
              <span className="mr-4 inline-block w-8 select-none text-right opacity-40">
                {lineIndex + 1}
              </span>
              <span className="flex-1">
                {line.tokens.map((token, tokenIndex) => (
                  <span
                    key={tokenIndex} // eslint-disable-line react/no-array-index-key -- static positional render of highlighted tokens; index key is fine
                    style={{
                      color: token.color,
                      fontStyle: token.fontStyle ? (token.fontStyle & 1 ? "italic" : "normal") : undefined,
                    }}
                  >
                    {token.content}
                  </span>
                ))}
              </span>
            </div>
          ))}
        </code>
      </pre>
    </div>
  );
}
