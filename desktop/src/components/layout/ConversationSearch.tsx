import { Search, X } from "lucide-react";
import { useEffect, useRef } from "react";
import { useTranslation } from "react-i18next";
import { cn } from "../../lib/cn";

/**
 * A compact, inline text search for the activity rail conversation list.
 *
 * Opens on demand so the rail keeps its narrow footprint, then filters rows as
 * the user types. Escape closes and clears the query.
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
          "inline-flex size-8 shrink-0 items-center justify-center rounded-md text-ink-muted transition-colors hover:bg-surface-subtle hover:text-ink-soft",
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

  return (
    <div
      className={cn(
        "flex h-8 flex-1 items-center gap-1 rounded-md border border-line-soft bg-surface px-2",
        className,
      )}
    >
      <Search className="size-3.5 shrink-0 text-ink-muted" />
      <input
        ref={inputRef}
        aria-label={t("activityRail.searchConversations")}
        autoCapitalize="none"
        autoCorrect="off"
        className="min-w-0 flex-1 bg-transparent text-sm text-ink placeholder:text-ink-muted focus:outline-none"
        onChange={event => onQueryChange(event.target.value)}
        placeholder={t("activityRail.searchConversations")}
        spellCheck={false}
        type="text"
        value={query}
      />
      {query.length > 0 && (
        <button
          aria-label={t("activityRail.clearSearch")}
          className="inline-flex size-5 shrink-0 items-center justify-center rounded text-ink-muted hover:bg-surface-subtle hover:text-ink-soft"
          onClick={() => onQueryChange("")}
          title={t("activityRail.clearSearch")}
          type="button"
        >
          <X className="size-3" />
        </button>
      )}
      <button
        aria-label={t("activityRail.closeSearch")}
        className="inline-flex size-5 shrink-0 items-center justify-center rounded text-ink-muted hover:bg-surface-subtle hover:text-ink-soft"
        onClick={onClose}
        title={t("activityRail.closeSearch")}
        type="button"
      >
        <X className="size-3.5" />
      </button>
    </div>
  );
}
