import { createContext, use } from "react";

/**
 * Marks a markdown block as the still-growing tail of a streaming reply.
 * Expensive renderers (shiki code highlighting) degrade to plain text inside
 * a live block: re-tokenizing a growing code block every 220ms poll tick is
 * O(block) per tick → O(n²) over a reply. Once the segment closes (or the run
 * settles), the same content re-renders fully highlighted.
 */
const LiveMarkdownContext = createContext(false);

export const LiveMarkdownProvider = LiveMarkdownContext.Provider;

/** True when this block is the live tail of a streaming reply. */
export function useLiveMarkdown(): boolean {
  return use(LiveMarkdownContext);
}
