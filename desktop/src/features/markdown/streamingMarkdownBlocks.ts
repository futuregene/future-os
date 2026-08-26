import remarkGfm from "remark-gfm";
import remarkMath from "remark-math";
import remarkParse from "remark-parse";
import { unified } from "unified";

export interface StreamingMarkdownBlock {
  /** Stable source offset used as the renderer key while later text grows. */
  start: number;
  /** Exact Markdown source for this top-level slice. */
  content: string;
  /** Only the final block of an in-flight reply remains mutable. */
  live: boolean;
}

interface StreamingMarkdownRoot {
  children: Array<{
    position?: { start?: { offset?: number } };
    type: string;
  }>;
}

const streamingMarkdownProcessor = unified().use(remarkParse).use(remarkMath).use(remarkGfm);

/**
 * Desktop-only projection for the growing Markdown renderer. Completed source
 * blocks stay immutable while the final block remains mutable. Reference-style
 * definitions can affect any block, so those documents remain whole.
 */
export function splitStreamingMarkdown(raw: string, live: boolean): StreamingMarkdownBlock[] {
  if (!raw)
    return [];

  const tree = streamingMarkdownProcessor.parse(raw) as unknown as StreamingMarkdownRoot;
  if (tree.children.some(node => node.type === "definition"))
    return [{ content: raw, live, start: 0 }];

  const starts = tree.children
    .map(node => node.position?.start?.offset)
    .filter((offset): offset is number => typeof offset === "number");
  if (starts.length <= 1)
    return [{ content: raw, live, start: 0 }];

  const blocks: StreamingMarkdownBlock[] = [];
  for (let index = 0; index < starts.length; index++) {
    const start = index === 0 ? 0 : starts[index]!;
    const end = starts[index + 1] ?? raw.length;
    if (end <= start)
      /* v8 ignore next -- remark emits strictly increasing, in-range top-level
         offsets, so a non-positive-length block never materializes */
      continue;
    blocks.push({
      content: raw.slice(start, end),
      live: live && index === starts.length - 1,
      start,
    });
  }
  return blocks.length > 0 ? blocks : [{ content: raw, live, start: 0 }];
}
