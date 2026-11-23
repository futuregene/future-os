import { useMemo } from "react";
import { useTranslation } from "react-i18next";
import { CopyButton } from "../../../components/ui/CopyButton";
import { useCopyState } from "../../../components/ui/useCopyState";
import { HighlightedCode } from "../HighlightedCode";
import { useLiveMarkdown } from "../LiveMarkdownContext";
import { useCodeHighlighter } from "../useCodeHighlighter";

export function CodeBlock({
  code,
  language,
  /**
   * Fold long lines instead of scrolling them sideways.
   *
   * A code block's layout is usually its meaning — box drawing, an aligned
   * table, a TUI capture all scramble when a line folds — so the `<pre>` pans
   * horizontally. A `futureos-choice` fence that the parser could not draw is
   * the exception (design `docs/design/choice-cards.zh-CN.md` §3 tolerance
   * table): its lines are `option: id | label | note` prose, and the reader is
   * meant to read the options they were offered, not to pan for them.
   */
  wrap = false,
}: {
  code: string;
  language?: string;
  wrap?: boolean;
}) {
  const { t } = useTranslation("markdown");
  const { copiedKey, copy } = useCopyState();
  const { highlight, isLoaded } = useCodeHighlighter();
  // The live tail of a streaming reply renders plain: re-tokenizing a growing
  // code block on every 220ms poll tick is O(block) per tick → O(n²) over the
  // reply. Full highlighting returns when the segment closes / the run settles.
  const live = useLiveMarkdown();
  const highlighted = useMemo(
    () => (live ? null : highlight(code, language)),
    [highlight, code, language, live],
  );
  // The fence's own language label is chrome for the parser: a reader who has
  // never seen the design gains nothing from the word `futureos-choice` sitting
  // above their options.
  const label = wrap ? undefined : language;
  const frame = wrap
    ? "overflow-hidden whitespace-pre-wrap break-words rounded-lg border border-line-soft bg-code-surface p-3 pr-11 text-xs leading-5"
    : "overflow-auto rounded-lg border border-line-soft bg-code-surface p-3 pr-11 text-xs leading-5";

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
        <pre className={`${frame} text-ink`}>
          {label ? <div className="mb-2 text-2xs text-ink-muted">{label}</div> : null}
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
      <pre className={frame} style={{ color: highlighted.fgColor }}>
        {label ? <div className="mb-2 text-2xs opacity-60">{label}</div> : null}
        <code>
          <HighlightedCode code={code} highlighted={highlighted} />
        </code>
      </pre>
    </div>
  );
}
