/**
 * Settings → Keyboard shortcuts.
 *
 * One table, read from `features/keyboard/catalog`: what each command is, what it
 * is bound to now, and whether that is still the default. The page owns no
 * binding of its own — it reads the live store and writes through it, which is
 * why a change here takes effect on the next key press: no restart, no save
 * button.
 *
 * The fixed keys are listed at the bottom and cannot be changed. They are not
 * decoration: someone deciding what to bind needs to see what Enter, Escape and
 * the transcript already answer, or the table looks emptier than the keyboard is.
 */
import type { CommandId } from "../keyboard/catalog";
import { RotateCcw, Search } from "lucide-react";
import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "../../components/ui/Button";
import { IconButton } from "../../components/ui/IconButton";
import { TextInput } from "../../components/ui/TextInput";
import { formatBinding, keyLabel, parseBinding } from "../../lib/keybinding";
import { isMacOS } from "../../lib/platform";
import { aliasBindingTexts, CATEGORIES, COMMANDS, FIXED_KEYS, scopeOf } from "../keyboard/catalog";
import { bindingLabel, isMultiKeyCommand, resetAllCommandBindings, resetCommandBinding, useShortcutOverrides } from "../keyboard/shortcuts";
import { SettingsList, SettingsRow, SettingsSection } from "./SettingsPrimitives";
import { ShortcutRecorder } from "./ShortcutRecorder";

export function ShortcutsPage() {
  const { t } = useTranslation("shortcuts");
  const overrides = useShortcutOverrides();
  const [query, setQuery] = useState("");
  const [recording, setRecording] = useState<string | null>(null);
  const isMac = isMacOS;

  // The three levels, spelled with the *live* bindings: a fixed sentence naming
  // ⌘⇧[ would keep naming it after the user rebound it.
  const pair = (previous: CommandId, next: CommandId) => `${bindingLabel(previous, overrides, isMac)} / ${bindingLabel(next, overrides, isMac)}`;
  const levels = t("page.levels", {
    coarse: pair("group.previous", "group.next"),
    fine: pair("thread.previous", "thread.next"),
    questions: `${keyLabel("PageUp", isMac)} / ${keyLabel("PageDown", isMac)}`,
  });

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

  return (
    <div className="space-y-6">
      <SettingsSection>
        {/* The three levels, then the search box: the sentences are part of the
            page's own explanation, and a section keeps its `description` only
            when it has a title — so they are content here. */}
        <p className="px-1 text-xs leading-5 text-ink-muted">{levels}</p>
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
        {/* Said once, not on thirteen rows: the terminal's rule is a rule about
            the *terminal*, and repeating it under every command reads as noise. */}
        <p className="px-1 text-xs leading-5 text-ink-muted">{t("page.terminalBlocked")}</p>
      </SettingsSection>

      {matches.length === 0
        ? <p className="px-1 text-sm text-ink-muted">{t("page.noMatches", { query })}</p>
        : null}

      {CATEGORIES.map((category) => {
        const rows = matches.filter(spec => spec.category === category);
        if (rows.length === 0)
          return null;
        // A group made only of mode commands says so once, at the top: every row
        // repeating "only while the panel is open" is noise, and a reader
        // deciding what to bind needs it exactly once.
        const scoped = rows.some(spec => scopeOf(spec) !== "global");
        return (
          <SettingsSection
            description={scoped ? t(category === "vibe" ? "page.scopedVibe" : "page.scopedGlobal") : undefined}
            key={category}
            title={t(`categories.${category}`)}
          >
            <SettingsList>
              {rows.map((spec) => {
                const customized = spec.id in overrides;
                return (
                  <SettingsRow
                    key={spec.id}
                    title={t(`commands.${spec.id}`)}
                  >
                    <div className="flex items-center gap-1">
                      {customized
                        ? (
                            <IconButton
                              className="size-7"
                              icon={<RotateCcw className="size-3.5" />}
                              label={t("page.reset")}
                              onClick={() => resetCommandBinding(spec.id)}
                            />
                          )
                        : null}
                      {isMultiKeyCommand(spec.id)
                        ? (
                            // Nine keys, one command: recording one of them would be a
                            // question the page cannot ask. Shown, not editable.
                            <span
                              className="rounded border border-line-soft bg-surface-subtle px-1.5 py-0.5 font-mono text-xs text-ink-soft"
                              title={t("page.multiKey")}
                            >
                              {bindingLabel(spec.id, overrides, isMac)}
                            </span>
                          )
                        : (
                            <ShortcutRecorder
                              armed={recording === spec.id}
                              command={spec.id}
                              onArmedChange={(armed) => {
                                setRecording(armed ? spec.id : null);
                              }}
                            />
                          )}
                      {/* The device's own keys. They stay on this command whatever the
                          reader binds above, so they are shown, not recorded. */}
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
              })}
            </SettingsList>
          </SettingsSection>
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
