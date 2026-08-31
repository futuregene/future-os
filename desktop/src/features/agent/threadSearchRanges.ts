interface TextRun {
  end: number;
  node: Text;
  start: number;
}

export interface ThreadTextRangeResult {
  hasMore: boolean;
  ranges: Range[];
}

/**
 * Find case-insensitive text matches in rendered thread content and map them
 * back to DOM ranges. Keeping this DOM-based makes search match what the user
 * can actually see, including rendered Markdown, code, and tool activity.
 */
export function findThreadTextRanges(root: HTMLElement, query: string, limit = Number.POSITIVE_INFINITY): ThreadTextRangeResult {
  if (!query)
    return { hasMore: false, ranges: [] };

  const runs: TextRun[] = [];
  let text = "";
  let previousMessageId: string | null = null;
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT, {
    acceptNode(node) {
      const parent = node.parentElement;
      if (!parent || !node.textContent || parent.closest("[data-thread-search-ignore], script, style"))
        return NodeFilter.FILTER_REJECT;
      return NodeFilter.FILTER_ACCEPT;
    },
  });

  let node = walker.nextNode();
  while (node) {
    const value = node.textContent ?? "";
    const messageId = node.parentElement?.closest<HTMLElement>("[data-message-id]")?.dataset.messageId ?? null;
    if (previousMessageId !== null && messageId !== previousMessageId)
      text += "\n";
    const start = text.length;
    text += value;
    runs.push({ end: text.length, node: node as Text, start });
    previousMessageId = messageId;
    node = walker.nextNode();
  }

  const ranges: Range[] = [];
  const matcher = new RegExp(escapeRegExp(query), "giu");
  let match = matcher.exec(text);
  let startRunIndex = 0;
  let endRunIndex = 0;
  while (match && ranges.length < limit) {
    const matchStart = match.index;
    const matchEnd = matchStart + match[0].length;
    while (runs[startRunIndex] && matchStart >= runs[startRunIndex]!.end)
      startRunIndex += 1;
    endRunIndex = Math.max(endRunIndex, startRunIndex);
    while (runs[endRunIndex] && matchEnd > runs[endRunIndex]!.end)
      endRunIndex += 1;
    const startRun = runs[startRunIndex];
    const endRun = runs[endRunIndex];
    if (startRun && endRun) {
      const range = document.createRange();
      range.setStart(startRun.node, matchStart - startRun.start);
      range.setEnd(endRun.node, matchEnd - endRun.start);
      ranges.push(range);
    }
    match = matcher.exec(text);
  }
  return { hasMore: match !== null, ranges };
}

function escapeRegExp(value: string): string {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}
