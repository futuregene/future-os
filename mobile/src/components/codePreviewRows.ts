export interface CodePreviewRow { text: string; continuation: boolean }

/** Bounded visual chunks, not one object per source line (a minified file or a
 * million empty lines must both remain bounded). Joining text reproduces the
 * source exactly; full-source copying never includes the continuation marker. */
export function codePreviewRows(code: string): CodePreviewRow[] {
  const rows: CodePreviewRow[] = [];
  let start = 0;
  do {
    let end = Math.min(code.length, start + 2048);
    let lastBreak = -1;
    let cursor = start;
    for (let line = 0; line < 32; line++) {
      const next = code.indexOf("\n", cursor);
      if (next < 0 || next >= end) break;
      lastBreak = next; cursor = next + 1;
      if (line === 31) end = cursor;
    }
    if (end < code.length && lastBreak >= start) end = lastBreak + 1;
    else if (end < code.length && /[\uD800-\uDBFF]/.test(code[end - 1]!)) end++;
    rows.push({ text: code.slice(start, end), continuation: start > 0 && code[start - 1] !== "\n" });
    start = end;
  } while (start < code.length);
  return rows;
}
