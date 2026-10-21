import type { RemoteReferenceOption } from "./remoteSessionReference";
import { MessagesSquare } from "lucide-react";
import { useEffect, useRef } from "react";

/**
 * The `#` conversation menu: one list, newest activity first.
 *
 * The same panel as the `/` menu beside it (`RemoteSlashMenu`), because it is
 * the same kind of thing — a list the draft opened, drawn above the box at the
 * foot of the window — and the same shape as this app's own `#` menu: no
 * workspace headings (recency is the list's one order), the workspace named on
 * the row, and a title over the panel so an empty draft that merely typed `#`
 * still says what it is offering.
 *
 * Rows are chosen with the mouse or the keyboard; the composer owns the
 * highlight so the two agree (the local composer keeps them in step the same
 * way).
 */
export function RemoteSessionMenu({
  activeIndex,
  emptyLabel,
  items,
  onChoose,
  title,
}: {
  activeIndex: number;
  emptyLabel: string;
  items: RemoteReferenceOption[];
  onChoose: (item: RemoteReferenceOption) => void;
  title: string;
}) {
  // Keep the keyboard-highlighted row visible while the list scrolls.
  const listRef = useRef<HTMLDivElement | null>(null);
  useEffect(() => {
    listRef.current
      ?.querySelector(`[data-menu-index="${activeIndex}"]`)
      ?.scrollIntoView({ block: "nearest" });
  }, [activeIndex]);

  return (
    <div
      className="absolute bottom-full left-1 mb-2 max-h-72 w-72 overflow-y-auto rounded-lg border border-line bg-surface p-1 shadow-panel"
      ref={listRef}
      role="listbox"
    >
      <div className="px-2 py-1 text-2xs font-medium text-ink-muted">{title}</div>
      {items.length === 0
        ? <div className="px-2 py-2 text-sm text-ink-muted">{emptyLabel}</div>
        : null}
      {items.map((item, index) => (
        <button
          aria-selected={index === activeIndex}
          className={`flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm ${
            index === activeIndex ? "bg-surface-subtle text-ink" : "text-ink-soft hover:bg-surface-subtle hover:text-ink"
          }`}
          data-menu-index={index}
          key={item.sessionId}
          // Keep the field's caret: the reference is inserted where the word was
          // typed, and the message continues from there. Without this the click
          // moves focus to the button and the next keystroke is lost.
          onMouseDown={event => event.preventDefault()}
          onClick={() => onChoose(item)}
          role="option"
          type="button"
        >
          <MessagesSquare aria-hidden className="size-4 shrink-0 text-ink-soft" />
          <span className="min-w-0 flex-1 truncate">{item.title}</span>
          {item.workspace
            ? (
                <span className="max-w-[40%] shrink-0 truncate text-xs text-ink-muted" data-session-workspace>
                  {item.workspace}
                </span>
              )
            : null}
        </button>
      ))}
    </div>
  );
}
