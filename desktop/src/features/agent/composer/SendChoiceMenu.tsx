import { ArrowUp } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { SelectMenu, SelectMenuItem } from "../../../components/ui/SelectMenu";
import { eventCode } from "../../../lib/keybinding";
import { useCommittedRef } from "../../../lib/useCommittedRef";

interface Props {
  /** How many messages are already waiting, which decides the second row's wording. */
  count: number;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onEnqueue: () => void;
  onInterruptAndSend: () => void;
  disabled?: boolean;
}

/**
 * The send button while a reply streams: instead of a Stop that would throw the
 * draft away, it asks what *this* send means — hold the draft in the queue, or
 * interrupt the run and send it now (see `Composer.onEnqueue`).
 *
 * Both answers are reachable without a pointer, because the reader is typing and
 * the pointer is not where their hand is. The caret stays in the field — the
 * draft must keep being editable, and focusing the menu would take the box away
 * — so this listens on the window in the **capture** phase and takes `↑`/`↓` and
 * `Enter` before the editor answers them: in the box `↑`/`↓` recall a sent
 * message and `Enter` is a send, and either would be the wrong answer while a
 * choice is on screen. The menu is opened by `Enter`, so the first row starts
 * selected and the same key confirms the reversible option — the one that spends
 * nothing and loses nothing.
 *
 * The modifiers keep their meaning: `⇧`/`⌘`/`⌥`/`⌃` with a key is a text-editing
 * gesture (select, word-move, `⇧Enter` for a newline), never a menu key. A chord
 * that opens no menu row must not also pick one.
 */
export function SendChoiceMenu({ count, open, onOpenChange, onEnqueue, onInterruptAndSend, disabled }: Props) {
  const { t } = useTranslation("agent");
  // Which row a `Enter` would run. Kept in state for the highlight and in a ref
  // for the key handler, which must read the row the reader sees rather than the
  // one some earlier render captured.
  const [row, setRow] = useState(0);
  const rowRef = useRef(0);
  // The rows in render order: index positions are what the arrows walk through.
  const actions = useCommittedRef<readonly (() => void)[]>([onEnqueue, onInterruptAndSend]);

  useEffect(() => {
    if (!open)
      return;
    // Opening lands on the first row, whichever row was picked last time.
    rowRef.current = 0;
    setRow(0);

    function onKeyDown(event: KeyboardEvent) {
      // Somebody closer to the caret already answered it, or an input method is
      // composing: `Enter` there confirms the candidates, it does not pick a row.
      if (event.defaultPrevented || event.isComposing)
        return;
      if (event.metaKey || event.ctrlKey || event.altKey || event.shiftKey)
        return;
      const rows = actions.current;
      const key = eventCode(event);
      const take = () => {
        // The editor behind this menu answers these same keys, so this reading
        // has to win — otherwise one press would both pick a row and send the
        // draft, or walk the history underneath the open menu.
        event.preventDefault();
        event.stopPropagation();
      };
      if (key === "ArrowDown" || key === "ArrowUp") {
        take();
        const delta = key === "ArrowDown" ? 1 : -1;
        rowRef.current = (rowRef.current + delta + rows.length) % rows.length;
        setRow(rowRef.current);
        return;
      }
      if (key === "Enter" || key === "NumpadEnter") {
        take();
        rows[rowRef.current]?.();
      }
    }

    window.addEventListener("keydown", onKeyDown, true);
    return () => window.removeEventListener("keydown", onKeyDown, true);
  }, [open, actions]);

  return (
    <SelectMenu
      align="right"
      open={open}
      onDismiss={() => onOpenChange(false)}
      panelClassName="w-72 overflow-hidden"
      trigger={(
        <button
          className="inline-flex size-7 items-center justify-center rounded-md bg-accent-fill text-white transition-colors hover:bg-accent-fill-hover disabled:bg-accent-fill-disabled"
          disabled={disabled}
          onClick={() => onOpenChange(!open)}
          type="button"
          aria-label={t("composer.sendWhileRunning.choose")}
          title={t("composer.sendWhileRunning.choose")}
        >
          <ArrowUp className="size-3.5" />
        </button>
      )}
    >
      <SelectMenuItem
        active={row === 0}
        className="py-1.5"
        onHover={() => setRow(0)}
        onSelect={onEnqueue}
        selected={false}
      >
        <span className="min-w-0 flex-1 space-y-0.5">
          <span className="block truncate font-medium leading-tight text-ink">
            {t("composer.sendWhileRunning.queue")}
          </span>
          <span className="block text-xs leading-tight text-ink-muted">
            {t("composer.sendWhileRunning.queueHint")}
          </span>
        </span>
      </SelectMenuItem>
      <SelectMenuItem
        active={row === 1}
        className="py-1.5"
        onHover={() => setRow(1)}
        onSelect={onInterruptAndSend}
        selected={false}
      >
        <span className="min-w-0 flex-1 space-y-0.5">
          <span className="block truncate font-medium leading-tight text-ink">
            {t(count > 0 ? "composer.sendWhileRunning.interruptAll" : "composer.sendWhileRunning.interrupt")}
          </span>
          <span className="block text-xs leading-tight text-ink-muted">
            {t(count > 0 ? "composer.sendWhileRunning.interruptAllHint" : "composer.sendWhileRunning.interruptHint")}
          </span>
        </span>
      </SelectMenuItem>
    </SelectMenu>
  );
}
