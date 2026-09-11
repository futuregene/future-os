import { memo } from "react";
import { cn } from "../../lib/cn";
import { STREAMED_BLOCK_CONTAINMENT } from "../markdown/LiveMarkdownContext";
import { StreamingMarkdownContent } from "../markdown/MarkdownContent";

/**
 * Dimmed, always-expanded display of the model's reasoning for one point in the
 * assistant reply's timeline. Rendered inline (in chronological order with text
 * and tool activity) and only when the "show thinking" setting is on.
 *
 * Memoized: a streaming reply re-renders on every push, but every reasoning
 * block except the growing tail has identical `text`/`live` props, so without
 * the memo each push rebuilt their whole element subtree. A long reasoning
 * reply is a hundred-odd blocks, so that walk is pure per-push waste.
 */
export const ThinkingBlock = memo(({
  text,
  workspaceId,
  live,
}: {
  text: string;
  workspaceId?: string | null;
  /** True while this reasoning block is the growing tail of a streaming reply. */
  live?: boolean;
}) => {
  return (
    <div
      className={cn(
        // A dimmed, borderless aside with a left rail — reads as reasoning, not
        // a filled content box (which is now reserved for code blocks).
        "border-l-2 border-line-soft pl-3 text-ink-muted",
        "**:text-ink-muted",
        // Same containment as the streamed text block (see LiveMarkdownContext):
        // a reasoning block is a tall column of text, and while it is the
        // growing tail its per-delta relayout would otherwise walk the whole
        // message's layout.
        live ? STREAMED_BLOCK_CONTAINMENT : "",
      )}
    >
      <StreamingMarkdownContent content={text} workspaceId={workspaceId} live={live} />
    </div>
  );
});
