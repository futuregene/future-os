import { useMemo } from "react";
import { mathHtml, useKatex } from "../lazyKatex";

/* eslint-disable react/dom-no-dangerously-set-innerhtml -- KaTeX renders
   HTML-escaped output; dangerouslySetInnerHTML is its documented render path */

interface MathBlockProps {
  code: string;
}

/**
 * Block-level math formula rendered with KaTeX.
 * Uses `displayMode` for centered, standalone equations.
 *
 * KaTeX loads on the first formula (`../lazyKatex`); until it lands the block
 * shows the formula's source, so the line keeps its height instead of popping in.
 */
export function MathBlock({ code }: MathBlockProps) {
  const katex = useKatex();
  const html = useMemo(() => (katex ? mathHtml(katex, code, true) : null), [katex, code]);

  return (
    <div className="my-3 overflow-x-auto py-2 text-center">
      {html === null
        ? <code className="font-mono text-sm text-ink-soft">{code}</code>
        : <div dangerouslySetInnerHTML={{ __html: html }} />}
    </div>
  );
}
