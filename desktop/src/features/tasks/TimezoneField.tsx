/**
 * The time-zone field: a closed row naming the zone, and a searchable list.
 *
 * The list is the whole reason this is not a `<select>`: the platform's zone
 * database is some four hundred entries, and a native dropdown of them forces a
 * reader to scroll past names in English that say nothing about the one thing
 * they need — how far the zone is from where they are. Each row therefore starts
 * with the name in the reader's own language and ends with the offset, and the
 * search box answers whatever they type (a city, a zone identifier, `+8`).
 *
 * It opens *inline* rather than as a popover: the form is a dialog with its own
 * scroll area, and a floating panel near the bottom of it would be clipped.
 */
import type { KeyboardEvent } from "react";
import { Check, ChevronDown, Search } from "lucide-react";
import { useCallback, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "../../components/ui/Button";
import { TextInput } from "../../components/ui/TextInput";
import { cn } from "../../lib/cn";
import { filterTimezoneChoices, timezoneChoices, timezoneLabel } from "./timezoneChoice";

interface TimezoneFieldProps {
  /** The stored value: an IANA name, or `""` for the machine's own zone. */
  value: string;
  onChange: (zone: string) => void;
  /** The UI language, which decides the zone names. */
  locale: string;
}

export function TimezoneField({ value, onChange, locale }: TimezoneFieldProps) {
  const { t } = useTranslation("tasks");
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const searchRef = useRef<HTMLInputElement>(null);

  const systemLabel = t("form.timezoneSystem");
  // One instant for the whole list: two rows must not disagree about the offset
  // because the clock ticked between them.
  const at = useMemo(() => new Date(), []);
  // Built when the list is first opened, not when the form renders. Naming four
  // hundred zones costs a formatting table each; a dialog that is waiting on
  // that would open late for a list most readers never scroll.
  const choices = useMemo(
    () => (open ? timezoneChoices(locale, at, systemLabel, value) : []),
    [at, locale, open, systemLabel, value],
  );
  const shown = useMemo(() => filterTimezoneChoices(choices, query), [choices, query]);
  const selected = timezoneLabel(value, locale, at, systemLabel);

  const choose = useCallback((zone: string) => {
    onChange(zone);
    setOpen(false);
    setQuery("");
  }, [onChange]);

  // Escape closes the list before it reaches the dialog, which would otherwise
  // read the same keypress as "discard the whole form".
  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.key !== "Escape")
      return;
    event.stopPropagation();
    setOpen(false);
    setQuery("");
  };

  return (
    <div className="block space-y-1.5" onKeyDown={onKeyDown}>
      <span className="text-xs text-ink-soft">{t("form.timezone")}</span>
      <div className="relative">
        <Button
          aria-expanded={open}
          aria-haspopup="listbox"
          className="w-full justify-between"
          data-testid="timezone-trigger"
          onClick={() => {
            setOpen(current => !current);
            setQuery("");
            // The search box is where a reader lands: the list is long and
            // typing is how they will narrow it.
            setTimeout(() => searchRef.current?.focus(), 0);
          }}
          type="button"
          variant="secondary"
        >
          <span className="truncate">{selected}</span>
          <ChevronDown aria-hidden className={cn("size-4 shrink-0 transition-transform", open && "rotate-180")} />
        </Button>
        {open
          ? (
              <div className="mt-1 overflow-hidden rounded-md border border-line-soft bg-surface shadow-panel">
                <div className="relative border-b border-line-soft p-2">
                  <Search aria-hidden className="pointer-events-none absolute left-4 top-1/2 size-3.5 -translate-y-1/2 text-ink-muted" />
                  <TextInput
                    aria-label={t("form.timezoneSearch")}
                    className="h-8 pl-8 text-xs"
                    data-testid="timezone-search"
                    onChange={event => setQuery(event.target.value)}
                    placeholder={t("form.timezoneSearch")}
                    ref={searchRef}
                    value={query}
                  />
                </div>
                <ul className="max-h-56 overflow-y-auto py-1" role="listbox">
                  {shown.length === 0
                    ? (
                        <li className="px-3 py-2 text-xs text-ink-muted">
                          {t("form.timezoneNoMatches", { query })}
                        </li>
                      )
                    : shown.map(choice => (
                        <li key={choice.zone || "__system"}>
                          <button
                            aria-selected={choice.zone === value}
                            className="flex w-full items-center gap-2 px-3 py-1.5 text-left text-xs transition-colors hover:bg-surface-subtle"
                            data-testid="timezone-option"
                            data-zone={choice.zone}
                            onClick={() => choose(choice.zone)}
                            role="option"
                            type="button"
                          >
                            <span className="min-w-0 flex-1 truncate">{choice.label}</span>
                            {choice.offset
                              ? (
                                  <span className="shrink-0 font-mono text-ink-muted">{choice.offset}</span>
                                )
                              : null}
                            {choice.zone === value
                              ? <Check aria-hidden className="size-3.5 shrink-0 text-ink-soft" />
                              : null}
                          </button>
                        </li>
                      ))}
                </ul>
              </div>
            )
          : null}
      </div>
      {/* The hint says what an unset choice means rather than leaving the user
          to guess whether it follows the machine or UTC. */}
      <p className="text-xs text-ink-muted">{t("form.timezoneHint")}</p>
    </div>
  );
}
