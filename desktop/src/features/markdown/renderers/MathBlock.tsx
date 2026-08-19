import katex from "katex";
import { useMemo } from "react";

/* eslint-disable react/dom-no-dangerously-set-innerhtml -- KaTeX renders
   HTML-escaped output; dangerouslySetInnerHTML is its documented render path */

interface MathBlockProps {
  code: string;
}

/**
 * Block-level math formula rendered with KaTeX.
 * Uses `displayMode` for centered, standalone equations.
 */
export function MathBlock({ code }: MathBlockProps) {
  const html = useMemo(() => {
    try {
      return katex.renderToString(code, {
        displayMode: true,
        throwOnError: false,
        trust: true,
        strict: false,
      });
    }
    catch {
      return `<span class="text-red-500">${escapeHtml(code)}</span>`;
    }
  }, [code]);

  return (
    <div
      className="my-3 overflow-x-auto py-2 text-center"
      dangerouslySetInnerHTML={{ __html: html }}
    />
  );
}

function escapeHtml(str: string): string {
  return str
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;");
}
