import type { RefObject } from "react";
import { Search, X } from "lucide-react";
import { useEffect, useRef } from "react";
import { useTranslation } from "react-i18next";
import { cn } from "../../lib/cn";

/**
 * The conversation search field above the activity rail's list.
 *
 * **Always on screen**, in the row the list's own controls occupy (see
 * `RailListTools` in `ActivityRail`). It used to be a glyph that expanded into
 * a field on click, on its own row, which cost a whole line for a control that
 * was used rarely and — worse — was invisible as a feature: nothing about a
 * small magnifier says "this list is searchable", so the affordance had to be
 * taught. A field that is always there needs no lesson, and it shares a row
 * with the list's actions instead of pushing a row of its own.
 *
 * Escape clears the query and hands the keyboard back (it does not remove the
 * field, because the field is not a mode). It only answers while this input has
 * focus: a stray Escape pressed in the transcript must not wipe a filter the
 * reader is not looking at.
 */
export function ConversationSearch({
  className,
  inputRef: forwardedInputRef,
  onQueryChange,
  query,
}: {
  className?: string;
  /**
   * The field's own input, when the owner needs to put the caret in it (the
   * rail's "search sessions" command does). The field is always on screen, so
   * that command is a focus, not an open.
   */
  inputRef?: RefObject<HTMLInputElement | null>;
  onQueryChange: (query: string) => void;
  query: string;
}) {
  const { t } = useTranslation("layout");
  const ownRef = useRef<HTMLInputElement>(null);
  const inputRef = forwardedInputRef ?? ownRef;

  useEffect(() => {
    if (query.length === 0)
      return;
    function handleKeyDown(event: KeyboardEvent) {
      if (event.key !== "Escape" || document.activeElement !== inputRef.current)
        return;
      event.stopPropagation();
      onQueryChange("");
      inputRef.current?.blur();
    }
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [inputRef, onQueryChange, query]);

  return (
    <div
      className={cn(
        "flex h-7 min-w-0 flex-1 items-center gap-1 rounded-md border border-line-soft bg-surface px-1.5",
        className,
      )}
      data-testid="rail-search"
    >
      <Search aria-hidden className="size-3.5 shrink-0 text-ink-muted" />
      <input
        ref={inputRef}
        aria-label={t("activityRail.searchConversations")}
        autoCapitalize="none"
        autoCorrect="off"
        className="min-w-0 flex-1 bg-transparent text-xs text-ink placeholder:text-ink-muted focus:outline-none"
        onChange={event => onQueryChange(event.target.value)}
        placeholder={t("activityRail.searchConversations")}
        spellCheck={false}
        type="text"
        value={query}
      />
      {query.length > 0 && (
        <button
          aria-label={t("activityRail.clearSearch")}
          className="inline-flex size-4 shrink-0 items-center justify-center rounded text-ink-muted hover:bg-surface-subtle hover:text-ink-soft"
          onClick={() => onQueryChange("")}
          title={t("activityRail.clearSearch")}
          type="button"
        >
          <X className="size-3" />
        </button>
      )}
    </div>
  );
}
