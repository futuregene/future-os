import { useMemo } from "react";
import { mathHtml, useKatex } from "../lazyKatex";

/* eslint-disable react/dom-no-dangerously-set-innerhtml -- KaTeX renders
   HTML-escaped output; dangerouslySetInnerHTML is its documented render path */

interface MathInlineProps {
  code: string;
}

/**
 * Inline math formula rendered with KaTeX.
 * Renders within the text flow (no displayMode).
 *
 * KaTeX loads on the first formula (`../lazyKatex`); until it lands the formula
 * shows its source, so the sentence keeps its shape instead of reflowing.
 */
export function MathInline({ code }: MathInlineProps) {
  const katex = useKatex();
  const html = useMemo(() => (katex ? mathHtml(katex, code, false) : null), [katex, code]);

  return html === null
    ? <code className="font-mono text-ink-soft">{code}</code>
    : <span className="inline-block" dangerouslySetInnerHTML={{ __html: html }} />;
}
