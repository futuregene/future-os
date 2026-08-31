import type { RefObject } from "react";
import { ArrowDown, ArrowUp, Search, X } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { IconButton } from "../../components/ui/IconButton";
import { findThreadTextRanges } from "./threadSearchRanges";

const MATCH_HIGHLIGHT = "thread-search-match";
const CURRENT_HIGHLIGHT = "thread-search-current";
/** Bound result objects and painted ranges for low-specificity queries in very long threads. */
const MAX_MATCHES = 300;
const MAX_PAINTED_MATCHES = 80;
const HIGHLIGHT_STYLES = `
  ::highlight(thread-search-match) { color: #0f172a; background: #fde047; }
  ::highlight(thread-search-current) { color: #0f172a; background: #fb923c; }
`;

interface HighlightRegistryLike {
  delete: (name: string) => boolean;
  get?: (name: string) => { clear?: () => void } | undefined;
  set: (name: string, highlight: Highlight) => void;
}

interface ThreadSearchProps {
  canLoadOlder: boolean;
  contentKey: unknown;
  onLoadOlder: () => void;
  rootRef: RefObject<HTMLElement | null>;
}

interface DeferredWork {
  firstFrame: number;
  secondFrame: number | null;
}

export function ThreadSearch({ canLoadOlder, contentKey, onLoadOlder, rootRef }: ThreadSearchProps) {
  const { t } = useTranslation("agent");
  const currentIndexRef = useRef(-1);
  const deferredWorkRef = useRef<DeferredWork | null>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const previousQueryRef = useRef("");
  const rangesRef = useRef<Range[]>([]);
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const [currentIndex, setCurrentIndex] = useState(-1);
  const [hasMoreMatches, setHasMoreMatches] = useState(false);
  const [matchCount, setMatchCount] = useState(0);

  const clearHighlights = useCallback(() => {
    const registry = getHighlightRegistry();
    registry?.get?.(MATCH_HIGHLIGHT)?.clear?.();
    registry?.get?.(CURRENT_HIGHLIGHT)?.clear?.();
    registry?.delete(MATCH_HIGHLIGHT);
    registry?.delete(CURRENT_HIGHLIGHT);
  }, []);

  const cancelDeferredWork = useCallback(() => {
    const work = deferredWorkRef.current;
    if (!work)
      return;
    window.cancelAnimationFrame(work.firstFrame);
    if (work.secondFrame !== null)
      window.cancelAnimationFrame(work.secondFrame);
    deferredWorkRef.current = null;
  }, []);

  // Interaction-triggered effects can run before the browser paints. Waiting
  // across two animation frames guarantees the open/close/input update gets a
  // paint before a full DOM scan or CSS Highlight registry mutation begins.
  const deferUntilAfterPaint = useCallback((callback: () => void) => {
    cancelDeferredWork();
    const work: DeferredWork = { firstFrame: 0, secondFrame: null };
    work.firstFrame = window.requestAnimationFrame(() => {
      work.secondFrame = window.requestAnimationFrame(() => {
        if (deferredWorkRef.current !== work)
          return;
        deferredWorkRef.current = null;
        callback();
      });
    });
    deferredWorkRef.current = work;
  }, [cancelDeferredWork]);

  const showMatch = useCallback((index: number, scroll = true) => {
    const ranges = rangesRef.current;
    const registry = getHighlightRegistry();
    clearHighlights();
    if (ranges.length === 0) {
      currentIndexRef.current = -1;
      setCurrentIndex(-1);
      return;
    }

    const normalized = (index + ranges.length) % ranges.length;
    const current = ranges[normalized]!;
    const paintStart = Math.max(0, normalized - Math.floor(MAX_PAINTED_MATCHES / 2));
    const paintEnd = Math.min(ranges.length, paintStart + MAX_PAINTED_MATCHES);
    const otherRanges = ranges.slice(paintStart, paintEnd).filter(range => range !== current);
    if (registry && typeof Highlight !== "undefined") {
      registry.set(MATCH_HIGHLIGHT, new Highlight(...otherRanges));
      registry.set(CURRENT_HIGHLIGHT, new Highlight(current));
    }
    currentIndexRef.current = normalized;
    setCurrentIndex(normalized);
    if (scroll) {
      const target = current.startContainer.parentElement;
      target?.scrollIntoView({ block: "center", inline: "nearest" });
    }
  }, [clearHighlights]);

  const recompute = useCallback(() => {
    const root = rootRef.current;
    const previousRange = rangesRef.current[currentIndexRef.current];
    const result = root && query
      ? findThreadTextRanges(root, query, MAX_MATCHES)
      : { hasMore: false, ranges: [] };
    const { hasMore, ranges } = result;
    rangesRef.current = ranges;
    setHasMoreMatches(hasMore);
    setMatchCount(ranges.length);
    const queryChanged = previousQueryRef.current !== query;
    previousQueryRef.current = query;
    const preservedIndex = previousRange
      ? ranges.findIndex(range => range.startContainer === previousRange.startContainer
        && range.startOffset === previousRange.startOffset
        && range.endContainer === previousRange.endContainer
        && range.endOffset === previousRange.endOffset)
      : -1;
    const nextIndex = queryChanged
      ? 0
      : preservedIndex >= 0
        ? preservedIndex
        : Math.min(Math.max(currentIndexRef.current, 0), ranges.length - 1);
    showMatch(nextIndex, queryChanged && ranges.length > 0);
    if (query && !hasMore && canLoadOlder)
      onLoadOlder();
  }, [canLoadOlder, onLoadOlder, query, rootRef, showMatch]);

  useEffect(() => {
    const handleKeyDown = (event: KeyboardEvent) => {
      if ((event.ctrlKey || event.metaKey) && event.key.toLocaleLowerCase() === "f") {
        event.preventDefault();
        event.stopPropagation();
        setOpen(true);
        window.requestAnimationFrame(() => {
          inputRef.current?.focus();
          inputRef.current?.select();
        });
        return;
      }
      if (!open)
        return;
      if (event.key === "Escape") {
        event.preventDefault();
        event.stopPropagation();
        setOpen(false);
      }
    };
    window.addEventListener("keydown", handleKeyDown, { capture: true });
    return () => window.removeEventListener("keydown", handleKeyDown, { capture: true });
  }, [open]);

  useEffect(() => {
    if (!open) {
      deferUntilAfterPaint(clearHighlights);
      return cancelDeferredWork;
    }
    inputRef.current?.focus();
    deferUntilAfterPaint(recompute);
    return cancelDeferredWork;
  }, [cancelDeferredWork, clearHighlights, contentKey, deferUntilAfterPaint, open, recompute]);

  useEffect(() => () => {
    cancelDeferredWork();
    clearHighlights();
  }, [cancelDeferredWork, clearHighlights]);

  useEffect(() => {
    if (!open || !rootRef.current)
      return;
    let frame = 0;
    const observer = new MutationObserver(() => {
      window.cancelAnimationFrame(frame);
      frame = window.requestAnimationFrame(() => deferUntilAfterPaint(recompute));
    });
    observer.observe(rootRef.current, { characterData: true, childList: true, subtree: true });
    return () => {
      observer.disconnect();
      window.cancelAnimationFrame(frame);
    };
  }, [deferUntilAfterPaint, open, recompute, rootRef]);

  const move = useCallback((delta: number) => {
    if (rangesRef.current.length > 0)
      showMatch(currentIndexRef.current + delta);
  }, [showMatch]);

  return (
    <>
      <style>{HIGHLIGHT_STYLES}</style>
      {open
        ? (
            <div
              className="absolute right-6 top-4 z-30 w-85 overflow-hidden rounded-3xl border border-line-soft bg-surface shadow-dialog"
              data-thread-search-ignore="true"
            >
              <div className="flex h-14 items-center gap-3 px-4">
                <Search aria-hidden="true" className="size-5 shrink-0 text-ink-soft" />
                <input
                  ref={inputRef}
                  aria-label={t("thread.searchPlaceholder")}
                  autoCapitalize="none"
                  autoComplete="off"
                  autoCorrect="off"
                  className="min-w-0 flex-1 bg-transparent text-base text-ink outline-none placeholder:text-ink-muted"
                  onChange={(event) => {
                    // The expensive DOM scan is deferred, but results from the old
                    // query must stop being interactive and visible immediately.
                    // Keep the previous counters rendered until the replacement
                    // result is ready so typing never flashes through "0 results".
                    rangesRef.current = [];
                    currentIndexRef.current = -1;
                    setQuery(event.target.value);
                    // Removing two named registry entries is cheap and must not wait
                    // for requestAnimationFrame: a throttled WebView can otherwise
                    // leave the previous query painted for many seconds.
                    clearHighlights();
                  }}
                  onKeyDown={(event) => {
                    if (event.key !== "Enter")
                      return;
                    event.preventDefault();
                    move(event.shiftKey ? -1 : 1);
                  }}
                  placeholder={t("thread.searchPlaceholder")}
                  spellCheck={false}
                  type="text"
                  value={query}
                />
                <div className="h-7 w-px bg-line-soft" />
                <IconButton
                  className="size-8 text-ink"
                  icon={<X className="size-5" />}
                  label={t("thread.searchClose")}
                  onClick={() => setOpen(false)}
                  type="button"
                />
              </div>
              <div className="flex h-12 items-center border-t border-line-soft px-3">
                <IconButton
                  className="size-8 disabled:cursor-default disabled:opacity-35"
                  disabled={matchCount === 0}
                  icon={<ArrowUp className="size-5" />}
                  label={t("thread.searchPrevious")}
                  onClick={() => move(-1)}
                  type="button"
                />
                <IconButton
                  className="size-8 disabled:cursor-default disabled:opacity-35"
                  disabled={matchCount === 0}
                  icon={<ArrowDown className="size-5" />}
                  label={t("thread.searchNext")}
                  onClick={() => move(1)}
                  type="button"
                />
                <span className="ml-auto pr-2 text-sm tabular-nums text-ink-muted">
                  {t(hasMoreMatches ? "thread.searchResultsMore" : "thread.searchResults", {
                    current: currentIndex + 1,
                    total: matchCount,
                  })}
                </span>
              </div>
            </div>
          )
        : null}
    </>
  );
}

function getHighlightRegistry(): HighlightRegistryLike | null {
  if (typeof CSS === "undefined")
    return null;
  return (CSS as typeof CSS & { highlights?: HighlightRegistryLike }).highlights ?? null;
}
