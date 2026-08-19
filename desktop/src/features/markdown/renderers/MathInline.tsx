import { useMemo } from "react";
import katex from "katex";

interface MathInlineProps {
  code: string;
}

/**
 * Inline math formula rendered with KaTeX.
 * Renders within the text flow (no displayMode).
 */
export function MathInline({ code }: MathInlineProps) {
  const html = useMemo(() => {
    try {
      return katex.renderToString(code, {
        displayMode: false,
        throwOnError: false,
        trust: true,
        strict: false,
      });
    } catch {
      return `<span class="text-red-500">${escapeHtml(code)}</span>`;
    }
  }, [code]);

  return (
    <span
      className="inline-block"
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
