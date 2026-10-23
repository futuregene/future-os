/**
 * Settings → Keyboard shortcuts.
 *
 * One table, read from `features/keyboard/catalog`: what each command is, what it
 * is bound to now, and whether that is still the default. The page owns no
 * binding of its own — it reads the live store and writes through it, which is
 * why a change here takes effect on the next key press: no restart, no save
 * button.
 *
 * The table is split into the catalog's **sections**, most-used first, and a
 * section can be folded shut. The Vibe panel's keys start closed: there are some
 * thirty of them, they answer only while that panel is open, and a reader who
 * came here to change `⌘N` should not have to scroll past them. Everything the
 * app itself answers — and every key the reader has changed — stays in view.
 *
 * The fixed keys are listed at the bottom and cannot be changed. They are not
 * decoration: someone deciding what to bind needs to see what Enter, Escape and
 * the transcript already answer, or the table looks emptier than the keyboard is.
 */
import type { CommandSectionId, CommandSpec } from "../keyboard/catalog";
import { ChevronRight, RotateCcw, Search } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "../../components/ui/Button";
import { IconButton } from "../../components/ui/IconButton";
import { TextInput } from "../../components/ui/TextInput";
import { cn } from "../../lib/cn";
import { formatBinding, parseBinding } from "../../lib/keybinding";
import { isMacOS } from "../../lib/platform";
import { aliasBindingTexts, COMMANDS, FIXED_KEYS, scopeOf, sectionCommands, SECTIONS } from "../keyboard/catalog";
import { bindingLabel, isMultiKeyCommand, resetAllCommandBindings, resetCommandBinding, useShortcutOverrides } from "../keyboard/shortcuts";
import { SettingsList, SettingsRow, SettingsSection } from "./SettingsPrimitives";
import { ShortcutRecorder } from "./ShortcutRecorder";

export function ShortcutsPage() {
  const { t } = useTranslation("shortcuts");
  const overrides = useShortcutOverrides();
  const [query, setQuery] = useState("");
  const [recording, setRecording] = useState<string | null>(null);
  // The sections the reader has folded shut, by id. `SECTIONS` decides which ones
  // start that way; everything else is open until someone closes it.
  const [collapsed, setCollapsed] = useState<ReadonlySet<CommandSectionId>>(
    () => new Set(SECTIONS.filter(section => section.startsCollapsed).map(section => section.id)),
  );
  const isMac = isMacOS;

  const normalized = query.trim().toLowerCase();
  const matches = useMemo(
    () => COMMANDS.filter((spec) => {
      if (!normalized)
        return true;
      return t(`commands.${spec.id}`).toLowerCase().includes(normalized)
        || spec.id.toLowerCase().includes(normalized);
    }),
    [normalized, t],
  );
  const customizedCount = Object.keys(overrides).length;

  // A match folded inside a closed section reads as "no match": searching opens
  // every section, before the rows are filtered. A section the reader closes again
  // *during* the search stays closed — this runs on the query, not on every render.
  const searching = normalized.length > 0;
  useEffect(() => {
    if (searching)
      setCollapsed(new Set());
  }, [searching]);

  const toggleSection = (id: CommandSectionId) => {
    setCollapsed((current) => {
      const next = new Set(current);
      if (next.has(id))
        next.delete(id);
      else
        next.add(id);
      return next;
    });
  };

  const rowFor = (spec: CommandSpec) => {
    const id = spec.id;
    const customized = id in overrides;
    return (
      <SettingsRow
        key={id}
        title={t(`commands.${id}`)}
      >
        <div className="flex items-center gap-1">
          {customized
            ? (
                <IconButton
                  className="size-7"
                  icon={<RotateCcw className="size-3.5" />}
                  label={t("page.reset")}
                  onClick={() => resetCommandBinding(id)}
                />
              )
            : null}
          {isMultiKeyCommand(id)
            ? (
                // Nine keys, one command: recording one of them would be a
                // question the page cannot ask. Shown, not editable.
                <span
                  className="rounded border border-line-soft bg-surface-subtle px-1.5 py-0.5 font-mono text-xs text-ink-soft"
                  title={t("page.multiKey")}
                >
                  {bindingLabel(id, overrides, isMac)}
                </span>
              )
            : (
                <ShortcutRecorder
                  armed={recording === id}
                  command={id}
                  onArmedChange={(armed) => {
                    setRecording(armed ? id : null);
                  }}
                />
              )}
          {/* The device's own keys. They stay on this command whatever the reader
              binds above, so they are shown, not recorded. */}
          {aliasBindingTexts(spec).map((text) => {
            const parsed = parseBinding(text);
            return parsed
              ? (
                  <span
                    className="rounded border border-line-soft bg-surface-subtle px-1.5 py-0.5 font-mono text-xs text-ink-soft"
                    key={text}
                    title={t("page.deviceKey")}
                  >
                    {formatBinding(parsed, isMac)}
                  </span>
                )
              : null;
          })}
        </div>
      </SettingsRow>
    );
  };

  return (
    <div className="space-y-6">
      <SettingsSection>
        <div className="flex items-center gap-2">
          <div className="relative min-w-0 flex-1">
            <Search aria-hidden className="pointer-events-none absolute left-2.5 top-1/2 size-3.5 -translate-y-1/2 text-ink-muted" />
            <TextInput
              aria-label={t("page.search")}
              className="pl-8"
              onChange={event => setQuery(event.target.value)}
              placeholder={t("page.search")}
              value={query}
            />
          </div>
          <Button
            disabled={customizedCount === 0}
            onClick={() => {
              resetAllCommandBindings();
              setRecording(null);
            }}
            size="sm"
            variant="secondary"
          >
            {t("page.resetAll")}
          </Button>
        </div>
      </SettingsSection>

      {matches.length === 0
        ? <p className="px-1 text-sm text-ink-muted">{t("page.noMatches", { query })}</p>
        : null}

      {SECTIONS.map((section) => {
        const rows = sectionCommands(section).filter(spec => matches.includes(spec));
        if (rows.length === 0)
          return null;
        const open = !collapsed.has(section.id);
        // A group made only of mode commands says so once, at the top: every row
        // repeating "only while the panel is open" is noise, and a reader
        // deciding what to bind needs it exactly once.
        const scoped = rows.some(spec => scopeOf(spec) !== "global");
        // Counted, not listed: a folded section must never be able to hide a
        // customisation, so the number of changed rows is on the heading too.
        const changed = rows.filter(spec => spec.id in overrides).length;
        return (
          <section className="space-y-2.5" key={section.id}>
            <div className="flex items-end justify-between gap-3">
              <div className="min-w-0">
                <h3 className="text-sm font-semibold text-ink">
                  <button
                    aria-expanded={open}
                    className="-mx-1 flex items-center gap-1 rounded px-1 text-sm font-semibold text-ink transition-colors hover:bg-surface-subtle"
                    onClick={() => toggleSection(section.id)}
                    type="button"
                  >
                    <ChevronRight
                      aria-hidden
                      className={cn("size-3.5 shrink-0 text-ink-muted transition-transform", open && "rotate-90")}
                    />
                    {t(`sections.${section.id}`)}
                  </button>
                </h3>
                {scoped
                  ? (
                      <p className="mt-0.5 text-xs leading-5 text-ink-muted">
                        {t(section.id === "vibe" ? "page.scopedVibe" : "page.scopedGlobal")}
                      </p>
                    )
                  : null}
              </div>
              <div className="flex shrink-0 items-center gap-2 pb-0.5 text-xs text-ink-muted">
                {changed > 0 ? <span>{t("page.changedCount", { count: changed })}</span> : null}
                <span>{t("page.keyCount", { count: rows.length })}</span>
              </div>
            </div>
            {open ? <SettingsList>{rows.map(rowFor)}</SettingsList> : null}
          </section>
        );
      })}

      <SettingsSection description={t("fixed.description")} title={t("fixed.title")}>
        <SettingsList>
          {FIXED_KEYS.map(item => (
            <SettingsRow key={item.labelKey} title={t(item.labelKey)}>
              <span className="rounded border border-line-soft bg-surface-subtle px-1.5 py-0.5 font-mono text-xs text-ink-soft">
                {isMac ? item.keys.mac : item.keys.other}
              </span>
            </SettingsRow>
          ))}
        </SettingsList>
      </SettingsSection>
    </div>
  );
}
