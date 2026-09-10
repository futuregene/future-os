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

/**
 * CSS layout containment applied to a still-growing streamed block.
 *
 * While a reply streams, every pushed delta mutates the growing tail and the
 * browser re-computes preferred widths and re-lays-out the whole accumulated
 * message — the dominant frame-time cost in WebContent samples during a long
 * reasoning reply (`RenderBlock::layout`, `computePreferredLogicalWidths`,
 * `paintObject`, `performFlexLayout`). `contain: layout style` makes the live
 * block a layout boundary: its size no longer depends on its children, so the
 * dirty tail stops forcing its ancestors to re-measure the entire bubble.
 * `contain-intrinsic-size: auto none` keeps its last laid-out size as the
 * fallback so it doesn't collapse to zero height while contained.
 *
 * Paint is deliberately NOT contained: these are plain in-flow text blocks
 * with no absolute/fixed positioning, so nothing should ever paint outside
 * them, and clipping the growing tail (selection handles, a code block's
 * floating copy button) would be a regression for no layout benefit.
 */
export const STREAMED_BLOCK_CONTAINMENT = "[contain:layout_style] [contain-intrinsic-size:auto_none]";
