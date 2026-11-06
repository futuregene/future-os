import { Brain, ChevronDown, ChevronUp } from "lucide-react";
import { memo, useState } from "react";
import { useTranslation } from "react-i18next";
import { cn } from "../../lib/cn";
import { STREAMED_BLOCK_CONTAINMENT } from "../markdown/LiveMarkdownContext";
import { StreamingMarkdownContent } from "../markdown/MarkdownContent";

/**
 * Where a bodyless reasoning row's text stands.
 *
 * The lean lanes send a reasoning block's boundaries and never its body, so "no
 * text" means *not loaded yet* — a different state from "there is nothing to
 * show", and the only one that can fail.
 */
type FetchState
  = | { status: "idle" }
    | { status: "loading" }
    | { status: "loaded"; text: string }
    | { status: "unavailable" };

/**
 * Reasoning starts collapsed like mobile and can always be expanded in place.
 * The revealed text is indented like a tool row's revealed target — one step in,
 * with no rule of its own — so reasoning and tool work read the same way.
 *
 * A row that arrived without its body (a remote transcript reading the trimmed
 * lane) is still expandable: the first open asks `onLoad` for the text and shows
 * it. Only a row with neither a body nor a way to fetch one is a plain label —
 * a chevron that opens an empty box promises content that never arrives.
 */
/**
 * A row's own segment in, that block's text out: what `MessageBlock` builds from
 * the reply's slice order and hands to the rows below it, so a reasoning row
 * needs no run identity of its own.
 */
export type ReasoningLoader = (segmentId: string) => Promise<string | null>;

export const ThinkingBlock = memo(({
  text,
  workspaceId,
  live,
  onLoad,
}: {
  text: string;
  workspaceId?: string | null;
  live?: boolean;
  /**
   * Fetch this row's reasoning on demand. Unset for a local run (the body is
   * already here) and for a host that cannot answer the read.
   */
  onLoad?: () => Promise<string | null>;
}) => {
  const { t } = useTranslation("agent");
  const [expanded, setExpanded] = useState(false);
  const [fetched, setFetched] = useState<FetchState>({ status: "idle" });
  const hasText = text.trim().length > 0;
  // A body is what earns the chevron — either the one that came with the reply,
  // or the one this row can go and read.
  const disclosable = hasText || !!onLoad;
  const body = hasText ? text : fetched.status === "loaded" ? fetched.text : null;
  const label = t(live ? "activity.thinking" : "activity.thoughtCompleted");
  const Chevron = expanded ? ChevronUp : ChevronDown;

  const toggle = () => {
    if (expanded) {
      setExpanded(false);
      return;
    }
    setExpanded(true);
    if (hasText || !onLoad)
      return;
    if (fetched.status === "loading" || fetched.status === "loaded")
      return;
    setFetched({ status: "loading" });
    void onLoad()
      .then((loaded) => {
        const trimmed = loaded?.trim() ?? "";
        setFetched(trimmed.length > 0
          ? { status: "loaded", text: loaded ?? "" }
          : { status: "unavailable" });
      })
      .catch(() => setFetched({ status: "unavailable" }));
  };
  const header = (
    <>
      <Brain className={cn("size-3.5 shrink-0", live && "animate-pulse")} />
      <span>{label}</span>
      {disclosable ? <Chevron className="size-3 shrink-0" /> : null}
    </>
  );
  const note = body
    ? null
    : fetched.status === "loading"
      ? t("activity.thoughtLoading")
      : fetched.status === "unavailable"
        ? t("activity.thoughtUnavailable")
        : null;

  return (
    <div className="flex min-w-0 flex-col gap-1 text-13 leading-6 text-ink-muted">
      {disclosable
        ? (
            <button
              aria-expanded={expanded}
              className="flex max-w-full cursor-pointer items-center gap-2 self-start text-left hover:text-ink"
              onClick={toggle}
              type="button"
            >
              {header}
            </button>
          )
        : <div className="flex max-w-full items-center gap-2 self-start">{header}</div>}
      {expanded && body
        ? (
            <div
              className={cn(
                "pl-6 text-ink-muted **:text-ink-muted",
                live ? STREAMED_BLOCK_CONTAINMENT : "",
              )}
            >
              <StreamingMarkdownContent content={body} workspaceId={workspaceId} live={live} />
            </div>
          )
        : null}
      {expanded && note ? <div className="pl-6 text-ink-muted">{note}</div> : null}
    </div>
  );
});
