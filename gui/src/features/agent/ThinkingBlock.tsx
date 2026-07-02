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
        "rounded-lg border border-line-soft bg-surface-subtle px-3 py-2 text-ink-muted",
        "[&_*]:text-ink-muted",
      )}
    >
      <MarkdownContent content={text} workspaceId={workspaceId} />
    </div>
  );
}
