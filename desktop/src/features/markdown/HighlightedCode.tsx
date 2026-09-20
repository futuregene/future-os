import type { HighlightResult } from "./useCodeHighlighter";

/**
 * A `HighlightResult` rendered as colored spans. Callers own the surrounding
 * `<pre>` — their typography differs — and pass the original `code` too,
 * because Shiki's token stream drops line separators: re-inserting the source's
 * own ones keeps CRLF / CR files byte-identical to what was read.
 *
 * Plain-text callers render the source string directly instead; both paths are
 * one expression so highlighting can never alter what the reader sees, selects
 * or copies.
 */
export function HighlightedCode({ code, highlighted }: { code: string; highlighted: HighlightResult }) {
  const lineEndings = code.match(/\r\n|\r|\n/g) ?? [];
  return (
    <>
      {highlighted.lines.map((line, lineIndex) => (
        // eslint-disable-next-line react/no-array-index-key -- static positional render of highlighted code; lines never reorder
        <span key={lineIndex}>
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
          {lineEndings[lineIndex] ?? ""}
        </span>
      ))}
    </>
  );
}
