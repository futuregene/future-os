import { cn } from "../../lib/cn";
import { MarkdownContent } from "../markdown/MarkdownContent";

/**
 * Dimmed, always-expanded display of the model's reasoning for one point in the
 * assistant turn's timeline. Rendered inline (in chronological order with text
 * and tool activity) and only when the "show thinking" setting is on.
 */
export function ThinkingBlock({
  text,
  workspaceId,
}: {
  text: string;
  workspaceId?: string | null;
}) {
  return (
    <div
      className={cn(
        // A dimmed, borderless aside with a left rail — reads as reasoning, not
        // a filled content box (which is now reserved for code blocks).
        "border-l-2 border-line-soft pl-3 text-ink-muted",
        "[&_*]:text-ink-muted",
      )}
    >
      <MarkdownContent content={text} workspaceId={workspaceId} />
    </div>
  );
}
