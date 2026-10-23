import { Search, X } from "lucide-react";
import { useEffect, useRef } from "react";
import { useTranslation } from "react-i18next";
import { cn } from "../../lib/cn";

/**
 * The activity rail's conversation search: a glyph that opens into a field.
 *
 * **A magnifier, until it is clicked.** The rail is a narrow column and the list
 * below it is the point; a field that is always drawn spends a row on a control
 * most sessions never touch. So the row holds one small glyph, and clicking it
 * expands the field in place — the glyph's own place, so nothing moves.
 *
 * Escape closes and clears the query, because a closed field must not leave a
 * filter behind that the reader cannot see.
 *
 * There is **one** `×`, and what it does follows the state of the field: with a
 * query it clears it, with an empty field it closes the search. Two `×` sat side
 * by side before — "clear" appearing only once there was text — and in a rail
 * this narrow that read as one broken control rather than as two different ones.
 */
export function ConversationSearch({
  className,
  onClose,
  onQueryChange,
  open,
  onToggleOpen,
  query,
}: {
  className?: string;
  onClose: () => void;
  onQueryChange: (query: string) => void;
  open: boolean;
  onToggleOpen: () => void;
  query: string;
}) {
  const { t } = useTranslation("layout");
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (open)
      inputRef.current?.focus();
  }, [open]);

  useEffect(() => {
    function handleKeyDown(event: KeyboardEvent) {
      if (event.key === "Escape" && open) {
        event.stopPropagation();
        onClose();
      }
    }
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [onClose, open]);

  if (!open) {
    return (
      <button
        aria-label={t("activityRail.openSearch")}
        className={cn(
          // Sized and placed like the compact controls beside it, so the closed
          // state is one more glyph in the row rather than a gap.
          "inline-flex size-7 shrink-0 items-center justify-center rounded-md text-ink-muted transition-colors hover:bg-surface-subtle hover:text-ink-soft",
          className,
        )}
        onClick={onToggleOpen}
        title={t("activityRail.openSearch")}
        type="button"
      >
        <Search className="size-3.5" />
      </button>
    );
  }

  // A query means "clear"; an empty field means "close". One control, two
  // answers, and the label says which one this press will be.
  const clearing = query.length > 0;

  return (
    <div
      className={cn(
        // `min-w-0`: the field shares the row with the device picker, and a flex
        // item's default `min-width: auto` let the input push the row wider than
        // the rail (the magnifier then sat outside it).
        //
        // `h-7`, the height of the glyph it replaces. The field was `h-8` while
        // that glyph was `size-7`, so clicking the magnifier grew the row by 4px:
        // the list under it jumped, and the picker sharing the row re-centred 2px
        // down — read as the row jittering at the moment of the click. The rail
        // draws this band, the switch's band under it and the compact picker at
        // 28px, so a field taking the glyph's place takes its height too.
        "flex h-7 min-w-0 flex-1 items-center gap-1 rounded-md border border-line-soft bg-surface px-2",
        className,
      )}
    >
      <Search className="size-3.5 shrink-0 text-ink-muted" />
      <input
        ref={inputRef}
        aria-label={t("activityRail.searchConversations")}
        autoCapitalize="none"
        autoCorrect="off"
        // The field's text is list content, so it takes the rail's content tier
        // (`text-13`) — the same one its rows and the device picker beside it
        // use. It was `text-sm` while that picker was `text-xs`, and two sizes
        // 2px apart on one 28px row read as two controls that had not been
        // aligned rather than as two different kinds of control.
        className="min-w-0 flex-1 bg-transparent text-13 text-ink placeholder:text-ink-muted focus:outline-none"
        onChange={event => onQueryChange(event.target.value)}
        placeholder={t("activityRail.searchConversations")}
        spellCheck={false}
        type="text"
        value={query}
      />
      <button
        aria-label={clearing ? t("activityRail.clearSearch") : t("activityRail.closeSearch")}
        className="inline-flex size-5 shrink-0 items-center justify-center rounded text-ink-muted hover:bg-surface-subtle hover:text-ink-soft"
        onClick={clearing ? () => onQueryChange("") : onClose}
        title={clearing ? t("activityRail.clearSearch") : t("activityRail.closeSearch")}
        type="button"
      >
        <X className="size-3.5" />
      </button>
    </div>
  );
}
