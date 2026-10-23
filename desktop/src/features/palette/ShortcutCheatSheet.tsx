/**
 * The cheat sheet: every key that is live right now.
 *
 * `mod+/`, and it reads the **store**, not the defaults — `useShortcutOverrides`
 * subscribes it, so a rebinding appears here on the next render without the dialog
 * being reopened. That is the whole point of a cheat sheet in an app whose keys are
 * editable: a printed card would be a card of the old keys.
 *
 * A section per block, in the table's order — the same sections the settings page
 * uses, so the sheet and the page that edits it never list the keyboard in two
 * different orders. A command that is scoped to a mode says so on its group, not
 * on every row; a command the user unbound is simply absent (there is nothing to
 * press).
 *
 * The header's one button goes to the page that *edits* these binds. It runs the
 * `shortcuts.settings` **command** rather than opening Settings itself: the
 * command is in the table, so it is rebindable and reachable from the palette and
 * the menu, and a second "open settings on the shortcuts page" path here would be
 * a second thing to keep in step.
 */
import { useMemo } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "../../components/ui/Button";
import { Overlay } from "../../components/ui/Overlay";
import { isMacOS } from "../../lib/platform";
import { scopeOf, sectionCommands, SECTIONS } from "../keyboard/catalog";
import { hasCommandHandler, runCommand } from "../keyboard/commands";
import { bindingLabel, useShortcutOverrides } from "../keyboard/shortcuts";

export function ShortcutCheatSheet({ onClose, open }: { onClose: () => void; open: boolean }) {
  const { t } = useTranslation("shortcuts");
  // Subscribed, so the sheet follows a rebinding while it is open.
  const overrides = useShortcutOverrides();
  const isMac = isMacOS;

  const groups = useMemo(
    () => SECTIONS.map((section) => {
      const rows = sectionCommands(section)
        .map(spec => ({
          id: spec.id,
          keys: bindingLabel(spec.id, overrides, isMac),
          label: t(`commands.${spec.id}`),
          scoped: scopeOf(spec) !== "global",
        }))
        // An unbound command has no key, so it has no place on a cheat sheet.
        .filter(row => row.keys.length > 0);
      return { section, rows };
    }).filter(group => group.rows.length > 0),
    [isMac, overrides, t],
  );

  return (
    <Overlay onClose={onClose} open={open}>
      <section
        aria-label={t("cheatSheet.ariaLabel")}
        aria-modal="true"
        className="relative z-10 flex max-h-[80vh] w-full max-w-2xl flex-col overflow-hidden rounded-xl border border-line-soft bg-surface shadow-dialog"
        role="dialog"
      >
        <header className="flex shrink-0 items-center justify-between gap-3 border-b border-line-soft px-5 py-3">
          <div className="flex min-w-0 items-baseline gap-3">
            <h2 className="text-base font-semibold text-ink">{t("cheatSheet.title")}</h2>
            <span className="truncate text-xs text-ink-muted">{t("cheatSheet.hint")}</span>
          </div>
          {/* Only while the command has an owner: on the onboarding gate there is no
              settings page to reach, and a button that does nothing is worse than
              none. `available` is read per render, and this dialog renders on open. */}
          {hasCommandHandler("shortcuts.settings")
            ? (
                <Button
                  onClick={() => {
                    // Close first: the settings dialog is the next overlay, and two
                    // stacked the wrong way round is a dialog nobody can see.
                    onClose();
                    runCommand("shortcuts.settings");
                  }}
                  size="xs"
                  variant="secondary"
                >
                  {t("cheatSheet.edit")}
                </Button>
              )
            : null}
        </header>
        <div className="min-h-0 flex-1 overflow-y-auto px-5 py-3">
          {groups.map(group => (
            <div className="mb-4" key={group.section.id}>
              <div className="mb-1 flex items-baseline gap-2">
                <h3 className="text-xs font-semibold uppercase tracking-wide text-ink-muted">
                  {t(`sections.${group.section.id}`)}
                </h3>
                {group.rows.some(row => row.scoped)
                  ? (
                      <span className="text-2xs text-ink-muted">
                        {t(group.section.id === "vibe" ? "page.scopedVibe" : "page.scopedGlobal")}
                      </span>
                    )
                  : null}
              </div>
              <dl className="divide-y divide-line-soft">
                {group.rows.map(row => (
                  <div className="flex items-center justify-between gap-4 py-1" key={row.id}>
                    <dt className="min-w-0 truncate text-sm text-ink-soft">{row.label}</dt>
                    <dd className="shrink-0 rounded border border-line-soft bg-surface-subtle px-1.5 py-0.5 font-mono text-xs text-ink">
                      {row.keys}
                    </dd>
                  </div>
                ))}
              </dl>
            </div>
          ))}
        </div>
      </section>
    </Overlay>
  );
}
