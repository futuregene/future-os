import { Brain, ChevronDown, ChevronUp } from "lucide-react";
import { memo, useState } from "react";
import { useTranslation } from "react-i18next";
import { cn } from "../../lib/cn";
import { STREAMED_BLOCK_CONTAINMENT } from "../markdown/LiveMarkdownContext";
import { StreamingMarkdownContent } from "../markdown/MarkdownContent";

/** Reasoning starts collapsed like mobile and can always be expanded in place. */
export const ThinkingBlock = memo(({
  text,
  workspaceId,
  live,
}: {
  text: string;
  workspaceId?: string | null;
  live?: boolean;
}) => {
  const { t } = useTranslation("agent");
  const [expanded, setExpanded] = useState(false);
  const label = t(live ? "activity.thinking" : "activity.thoughtCompleted");
  const Chevron = expanded ? ChevronUp : ChevronDown;

  return (
    <div className="flex min-w-0 flex-col gap-1 text-[13px] leading-6 text-ink-muted">
      <button
        aria-expanded={expanded}
        className="flex max-w-full cursor-pointer items-center gap-2 self-start text-left hover:text-ink"
        onClick={() => setExpanded(value => !value)}
        type="button"
      >
        <Brain className={cn("size-3.5 shrink-0", live && "animate-pulse")} />
        <span>{label}</span>
        <Chevron className="size-3 shrink-0" />
      </button>
      {expanded
        ? (
            <div
              className={cn(
                "border-l-2 border-line-soft pl-3 text-ink-muted **:text-ink-muted",
                live ? STREAMED_BLOCK_CONTAINMENT : "",
              )}
            >
              <StreamingMarkdownContent content={text} workspaceId={workspaceId} live={live} />
            </div>
          )
        : null}
    </div>
  );
});
