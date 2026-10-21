/**
 * The command palette: one box that runs a command or opens a conversation.
 *
 * Open with `mod+k` (the `palette.open` command, rebindable like any other). It
 * is a modal, so the global dispatcher stands down behind it
 * (`components/ui/overlayStack` + `lib/keyboardOwners` both see it) and the arrow
 * keys and `↩` here belong to the list rather than to the transcript.
 *
 * The rows come from `paletteItems`; this file is the dialog, the filtering input
 * and the keyboard. A row either carries a command id — which this view runs
 * through the command registry, exactly as the dispatcher would — or its own
 * action, which is how "open this conversation" stays a caller concern.
 *
 * Commands that currently have no handler are not listed at all
 * (`hasCommandHandler`): the palette promises to do what it says, and a row that
 * does nothing is worse than a row that is not there — the same rule the
 * dispatcher applies to a key with no owner.
 */
import type { KeyboardEvent } from "react";
import type { CommandCategory } from "../keyboard/catalog";
import type { PaletteCommand, PaletteEntry, PaletteTarget } from "./paletteItems";
import { Search } from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Overlay } from "../../components/ui/Overlay";
import { cn } from "../../lib/cn";
import { hasCommandHandler, runCommand } from "../keyboard/commands";
import { flattenSections, paletteSections } from "./paletteItems";
import { isPaletteToggleKey } from "./paletteToggle";

export function CommandPalette({
  commands,
  onClose,
  open,
  targets,
}: {
  /**
   * Every command of the global keymap (minus the palette's own two). Which of
   * them can run right now is decided here, at open time — see `available`.
   */
  commands: readonly PaletteCommand[];
  onClose: () => void;
  open: boolean;
  targets: readonly PaletteTarget[];
}) {
  const { t } = useTranslation("palette");
  const { t: tShortcuts } = useTranslation("shortcuts");
  const [query, setQuery] = useState("");
  const [selected, setSelected] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLDivElement>(null);

  const labels = useMemo(() => ({
    commands: t("sections.commands"),
    // A category name comes from the shortcuts namespace: it is the same word the
    // settings page groups those commands under, so the two cannot drift.
    category: (category: CommandCategory) => tShortcuts(`categories.${category}`),
    conversations: t("sections.conversations"),
    workspaces: t("sections.workspaces"),
  }), [t, tShortcuts]);

  // Availability is decided **when the palette opens**, not when the shell last
  // rendered: handlers are registered by effects, so a list captured on a first
  // render would be a list of nothing. `open` is a dependency on purpose.
  // `open` is a dependency the linter calls unnecessary and this needs: the
  // command set must be re-read every time the dialog opens, because installing
  // handlers is an effect and reopening is the moment that can have changed.
  // eslint-disable-next-line react/exhaustive-deps
  const available = useMemo(() => commands.filter(command => hasCommandHandler(command.id)), [commands, open]);
  const sections = useMemo(
    () => paletteSections({ commands: available, labels, query, targets }),
    [available, labels, query, targets],
  );
  const rows = useMemo(() => flattenSections(sections), [sections]);

  // A new query re-ranks the list; the selection has to follow it or `↩` would
  // run whatever happened to be at the old index.
  useEffect(() => {
    setSelected(0);
  }, [query]);
  useEffect(() => {
    if (!open)
      return;
    setQuery("");
    setSelected(0);
    // An effect rather than an animation frame: the input is committed by the time
    // effects run, and a frame is one more thing a test has to flush.
    inputRef.current?.focus();
  }, [open]);
  // Keep the selected row in view while the arrows walk past the fold.
  useEffect(() => {
    const row = listRef.current?.querySelector<HTMLElement>(`[data-palette-index="${selected}"]`);
    // Optional-call: a renderer without layout (jsdom) has no scrollIntoView, and a
    // list that cannot scroll is not a reason to fail a keystroke.
    row?.scrollIntoView?.({ block: "nearest" });
  }, [selected]);

  const choose = useCallback((entry: PaletteEntry | undefined) => {
    if (!entry)
      return;
    // Close first: the command may open something of its own, and two overlays
    // stacked in the wrong order is a dialog the user cannot see.
    onClose();
    if (entry.commandId)
      runCommand(entry.commandId);
    else
      entry.run?.();
  }, [onClose]);

  function handleKeyDown(event: KeyboardEvent<HTMLInputElement>) {
    // The key that opened it closes it: a palette you cannot dismiss with the key
    // your fingers are already on is a palette you learn to avoid.
    if (isPaletteToggleKey(event)) {
      event.preventDefault();
      onClose();
      return;
    }
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault();
      if (rows.length === 0)
        return;
      const delta = event.key === "ArrowDown" ? 1 : -1;
      setSelected(current => (current + delta + rows.length) % rows.length);
      return;
    }
    if (event.key === "Enter") {
      event.preventDefault();
      choose(rows[selected]);
    }
  }

  return (
    <Overlay onClose={onClose} open={open}>
      <section
        aria-label={t("ariaLabel")}
        aria-modal="true"
        className="relative z-10 flex max-h-[70vh] w-full max-w-xl flex-col overflow-hidden rounded-xl border border-line-soft bg-surface shadow-dialog"
        role="dialog"
      >
        <div className="relative flex shrink-0 items-center border-b border-line-soft">
          <Search aria-hidden className="pointer-events-none absolute left-4 size-4 text-ink-muted" />
          <input
            aria-label={t("placeholder")}
            autoCapitalize="none"
            autoCorrect="off"
            className="h-12 w-full bg-transparent pl-11 pr-4 text-sm text-ink placeholder:text-ink-muted focus-visible:outline-none"
            data-palette-input="true"
            onChange={(event) => {
              setQuery(event.target.value);
            }}
            onKeyDown={handleKeyDown}
            placeholder={t("placeholder")}
            ref={inputRef}
            spellCheck={false}
            value={query}
          />
        </div>
        <div className="min-h-0 flex-1 overflow-y-auto py-1" ref={listRef}>
          {rows.length === 0
            ? <p className="px-4 py-6 text-sm text-ink-muted">{t("noMatches", { query })}</p>
            : null}
          {sections.map(section => (
            <div key={section.key}>
              <div className="px-4 pb-1 pt-2 text-xs font-medium uppercase tracking-wide text-ink-muted">
                {section.title}
              </div>
              {section.entries.map((entry) => {
                const index = rows.indexOf(entry);
                return (
                  <button
                    aria-selected={index === selected}
                    className={cn(
                      "flex w-full items-center gap-3 px-4 py-1.5 text-left text-sm",
                      index === selected ? "bg-surface-subtle text-ink" : "text-ink-soft hover:bg-surface-subtle",
                    )}
                    data-palette-index={index}
                    key={entry.key}
                    onClick={() => choose(entry)}
                    onMouseEnter={() => setSelected(index)}
                    role="option"
                    type="button"
                  >
                    <span className="min-w-0 flex-1 truncate">{entry.label}</span>
                    {entry.hint
                      ? <span className="shrink-0 truncate text-xs text-ink-muted">{entry.hint}</span>
                      : null}
                  </button>
                );
              })}
            </div>
          ))}
        </div>
        <div className="flex shrink-0 items-center gap-3 border-t border-line-soft px-4 py-1.5 text-xs text-ink-muted">
          <span>{t("footer.move")}</span>
          <span>{t("footer.run")}</span>
          <span>{t("footer.close")}</span>
        </div>
      </section>
    </Overlay>
  );
}
