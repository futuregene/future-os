import { Brain, ChevronDown, ChevronUp } from "lucide-react";
import { memo, useState } from "react";
import { useTranslation } from "react-i18next";
import { cn } from "../../lib/cn";
import { STREAMED_BLOCK_CONTAINMENT } from "../markdown/LiveMarkdownContext";
import { StreamingMarkdownContent } from "../markdown/MarkdownContent";

/** Collapsed reasoning matches mobile; the setting still gates its full content. */
export const ThinkingBlock = memo(({
  text,
  workspaceId,
  live,
  showContent = false,
  inSteps = false,
}: {
  text: string;
  workspaceId?: string | null;
  live?: boolean;
  showContent?: boolean;
  /** Revealed summary children belong to the left-aligned reading column. */
  inSteps?: boolean;
}) => {
  const { t } = useTranslation("agent");
  const [expanded, setExpanded] = useState(false);
  const open = showContent && expanded;
  const label = t(live ? "activity.thinking" : "activity.thoughtCompleted");
  const Chevron = open ? ChevronUp : ChevronDown;
  const header = (
    <>
      <Brain className={cn("size-3.5 shrink-0", live && "animate-pulse")} />
      <span>{label}</span>
    </>
  );

  return (
    <div className="flex min-w-0 flex-col gap-1 text-[13px] leading-6 text-ink-muted">
      {showContent
        ? (
            <button
              aria-expanded={open}
              className={cn("flex max-w-full cursor-pointer items-center gap-2 text-left hover:text-ink", !inSteps && !open ? "self-end" : "self-start")}
              onClick={() => setExpanded(value => !value)}
              type="button"
            >
              {header}
              <Chevron className="size-3 shrink-0" />
            </button>
          )
        : <div className={cn("flex items-center gap-2", !inSteps ? "self-end" : "self-start")}>{header}</div>}
      {open
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
