import type { VibeMenuRow } from "./menuNav";
import { useEffect, useRef, useState } from "react";
import { MenuPanel } from "../../components/ui/MenuPanel";
import { cn } from "../../lib/cn";
import { eventCode } from "../../lib/keybinding";
import { useKeyboardOwner } from "../../lib/keyboardOwners";
import { firstEnabled, nextEnabled } from "./menuNav";

/**
 * The panel's menu: the thing a remote control's `Menu` button opens.
 *
 * A remote has no pointer, so a menu is not a convenience here — it is how the
 * panel is configured at all. Three consequences shape it:
 *
 * - **One column, up and down.** `↑`/`↓` walk the rows; a row that holds a *value*
 *   (the layout, later a machine filter) is adjusted with `←`/`→` in place, which
 *   is the grammar every television's settings page already taught. A row never
 *   opens a second menu for a value.
 * - **`Esc` and `Back` close it, and `Menu` closes it too.** A key that opened
 *   something must never be the one key that cannot close it.
 * - **It holds the keyboard while it is up** (`useKeyboardOwner`), so a chord that
 *   would rearrange the panel behind it does nothing.
 *
 * Rows come from the caller: the panel decides what is configurable, this component
 * decides how a menu behaves. A row with `onAdjust` is a choice; one without is an
 * action.
 */
export function VibeMenu({
  onClose,
  rows,
  title,
}: {
  onClose: () => void;
  rows: readonly VibeMenuRow[];
  title: string;
}) {
  useKeyboardOwner(true);
  const [index, setIndex] = useState(() => firstEnabled(rows, 0));
  const panelRef = useRef<HTMLDivElement>(null);

  // Take the keyboard: the menu is opened by a key, so the next key should land
  // here without anyone needing to press something intermediate.
  useEffect(() => {
    panelRef.current?.focus();
  }, []);

  /**
   * The keys, from the **window**, not from the panel.
   *
   * A remote control's button goes wherever focus happens to be, and the menu is
   * opened by a key while the caret may be in an editor two layers down. Listening
   * on the window while the menu is up (capture, so nothing else acts on the key
   * first) is what makes "the menu owns the keyboard" true regardless of focus —
   * and it is what lets the same press that opened the menu close it.
   */
  useEffect(() => {
    function onKeyDown(event: KeyboardEvent) {
      if (event.defaultPrevented)
        return;
      // A chord is not a menu key. `⌘⌥→` is the panel's "move this cell", and the
      // owner registration is what keeps it from running behind the menu — but the
      // menu must not *adjust* on it either, or one press would do both.
      if (event.metaKey || event.ctrlKey || event.altKey)
        return;
      function take() {
        event.preventDefault();
        event.stopPropagation();
      }
      // The same reading of "which key is this" the shortcut dispatcher uses: a
      // WebKit build that sends no `code` still names the key, and a synthetic
      // press (a screenshot harness, a test) frequently sends only `key`. Reading
      // `code` alone silently ignored `←`/`→` from the real page while every jsdom
      // test passed — the harness is what caught it.
      switch (eventCode(event)) {
        case "ArrowDown":
        case "ArrowUp":
          take();
          setIndex(current => nextEnabled(rows, current, eventCode(event) === "ArrowDown" ? 1 : -1));
          return;
        case "ArrowLeft":
        case "ArrowRight":
          take();
          rows[index]?.onAdjust?.(eventCode(event) === "ArrowRight" ? 1 : -1);
          return;
        case "Enter":
          take();
          if (!rows[index]?.disabled)
            rows[index]?.onRun();
          return;
        // The remote's own menu button: the key that opened it closes it.
        case "Escape":
        case "ContextMenu":
          take();
          onClose();
          break;
        default:
          break;
      }
    }
    window.addEventListener("keydown", onKeyDown, true);
    return () => window.removeEventListener("keydown", onKeyDown, true);
  }, [index, onClose, rows]);

  return (
    <div
      className="fixed inset-0 z-50 flex items-start justify-center bg-overlay/45 pt-[10vh]"
      // A pointer may also use this menu; the backdrop is what makes "click away"
      // work without a trap, and it closes on the way down so the click cannot
      // also reach a cell behind it.
      onMouseDown={onClose}
      role="presentation"
    >
      <MenuPanel
        aria-label={title}
        className="pointer-events-auto w-[min(30rem,calc(100vw-3rem))] overflow-hidden p-1 outline-none"
        onMouseDown={(event) => {
          // The backdrop's handler must not see a press on the panel itself.
          event.stopPropagation();
        }}
        ref={panelRef}
        role="menu"
        tabIndex={-1}
      >
        <div className="px-2 pb-1 pt-2 text-[11px] font-medium uppercase tracking-wide text-ink-faint">
          {title}
        </div>
        {rows.map((row, rowIndex) => (
          <MenuRowView
            key={row.id}
            onHover={() => setIndex(rowIndex)}
            onRun={() => {
              if (!row.disabled)
                row.onRun();
            }}
            row={row}
            selected={rowIndex === index}
          />
        ))}
      </MenuPanel>
    </div>
  );
}

function MenuRowView({
  row,
  selected,
  onHover,
  onRun,
}: {
  row: VibeMenuRow;
  selected: boolean;
  onHover: () => void;
  onRun: () => void;
}) {
  return (
    <>
      {row.group ? <div className="my-1 h-px bg-line-soft" role="separator" /> : null}
      <button
        aria-current={selected ? "true" : undefined}
        aria-disabled={row.disabled || undefined}
        className={cn(
          "flex w-full items-center gap-3 rounded-md px-2 py-1.5 text-left",
          row.disabled
            ? "cursor-default text-ink-faint"
            : selected ? "bg-surface-subtle text-ink" : "text-ink-soft hover:bg-surface-subtle",
        )}
        data-vibe-menu-row={row.id}
        onClick={onRun}
        onMouseEnter={onHover}
        role="menuitem"
        type="button"
      >
        <span className="min-w-0 flex-1">
          <span className="block truncate text-sm font-medium">{row.label}</span>
          {row.description
            ? <span className="block truncate text-xs text-ink-muted">{row.description}</span>
            : null}
        </span>
        {row.value
          ? (
              <span className="shrink-0 rounded border border-line-soft bg-surface px-1.5 py-0.5 font-mono text-[11px] text-ink-soft">
                {row.value}
              </span>
            )
          : null}
        {/* The horizontal pair, shown only on the row that answers it: a reader
            deciding between "←/→ changes this" and "←/→ does nothing" cannot be
            expected to try it. */}
        {row.onAdjust && !row.disabled
          ? <span aria-hidden className="shrink-0 font-mono text-[11px] text-ink-faint">← →</span>
          : null}
      </button>
    </>
  );
}
