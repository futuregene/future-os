import type { ThemePalette } from "../../lib/theme";
import { Check } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "../../components/ui/Button";
import { cn } from "../../lib/cn";
import { setThemePalette, THEME_PALETTES, useThemePalette } from "../../lib/theme";
import { SettingsRow } from "./SettingsPrimitives";

/**
 * The color-theme picker.
 *
 * A row of miniature windows rather than a `<Select>` of names, because a theme
 * name says nothing: "Nord" and "Gruvbox" are only distinguishable by looking at
 * them, and a user choosing a look is doing exactly that. This is how the apps
 * people compare us to do it (VS Code, Slack, Linear) — a preview you recognize
 * beats a list you have to try one by one.
 *
 * **Collapsed by default.** Fourteen cards are a wall of previews for a setting
 * most users touch once, and they push every row below them off the page. So
 * the row normally costs what any other setting costs — the active palette as a
 * small swatch, the same way the neighbouring rows show one value — and the
 * catalogue opens on request. The choice is unchanged: picking a card in the
 * open grid applies it at once, which is why the grid stays open afterwards
 * (nobody compares themes one reopening at a time).
 *
 * The component owns its whole row, because the row's *shape* follows the state:
 * beside the label while collapsed (a compact control, like a Select) and below
 * it once expanded (a grid as wide as the column).
 *
 * Each preview carries its own `data-palette`, which is why the palette rules in
 * `styles/globals.css` match a nested preview element as well as the root: the
 * preview paints itself in the palette it shows while the app keeps the one that
 * is actually active. The preview inherits whichever rendition (`.dark`) the
 * window is in, so in a dark window it shows that palette's dark self.
 */
export function ThemePalettePicker() {
  const { t } = useTranslation("settings");
  const active = useThemePalette();
  const [expanded, setExpanded] = useState(false);

  const toggle = (
    <Button
      aria-expanded={expanded}
      onClick={() => setExpanded(value => !value)}
      size="sm"
      variant="secondary"
    >
      {expanded
        ? t("theme.collapse")
        : t("theme.showAll", { count: THEME_PALETTES.length })}
    </Button>
  );

  return (
    <SettingsRow
      description={t("theme.description")}
      stacked={expanded}
      title={t("theme.title")}
    >
      {expanded
        ? (
            <div className="flex w-full flex-col gap-2.5">
              <div
                aria-label={t("theme.title")}
                className="grid w-full grid-cols-3 gap-2.5"
                role="radiogroup"
              >
                {THEME_PALETTES.map(palette => (
                  <PaletteCard
                    active={active === palette}
                    key={palette}
                    label={t(`theme.palettes.${palette}.name`)}
                    onSelect={() => setThemePalette(palette)}
                    description={t(`theme.palettes.${palette}.description`)}
                    palette={palette}
                  />
                ))}
              </div>
              {toggle}
            </div>
          )
        : (
            <div className="flex items-center gap-3">
              {/* The active palette, as the swatch the row reads at a glance —
                  the value of this setting, next to the way out of it. */}
              <span className="flex items-center gap-2" title={t(`theme.palettes.${active}.description`)}>
                <span data-palette={active}>
                  <PaletteSwatch />
                </span>
                <span className="text-sm text-ink">{t(`theme.palettes.${active}.name`)}</span>
              </span>
              {toggle}
            </div>
          )}
    </SettingsRow>
  );
}

/** One palette as a selectable card: the preview plus its name and check mark. */
function PaletteCard({
  active,
  description,
  label,
  onSelect,
  palette,
}: {
  active: boolean;
  /** Shown as the card's tooltip: why this theme exists, in one line. */
  description: string;
  label: string;
  onSelect: () => void;
  palette: ThemePalette;
}) {
  return (
    <button
      aria-checked={active}
      aria-label={label}
      className="group flex min-w-0 flex-col gap-1.5 rounded-lg text-left outline-none focus-visible:ring-2 focus-visible:ring-focus"
      data-palette={palette}
      onClick={onSelect}
      role="radio"
      title={description}
      type="button"
    >
      <span
        aria-hidden
        className={cn(
          "block w-full overflow-hidden rounded-lg border bg-canvas p-2 transition-colors",
          active
            ? "border-accent ring-2 ring-accent/40"
            : "border-line group-hover:border-ink-muted",
        )}
      >
        {/* The window: a surface card over the canvas, with a text bar and a
            primary-action pill, the three things a theme is judged by. */}
        <span className="flex h-12 w-full flex-col gap-1.5 rounded-md bg-surface p-1.5">
          <span className="flex items-center gap-1">
            <span className="size-1.5 rounded-full bg-accent" />
            <span className="h-1 w-5 rounded-full bg-ink" />
            <span className="ml-auto h-1 w-3 rounded-full bg-line" />
          </span>
          <span className="h-1 w-11/12 rounded-full bg-ink-muted" />
          <span className="mt-auto flex items-center gap-1">
            <span className="h-2.5 w-8 rounded-sm bg-accent-fill" />
            <span className="h-2.5 w-4 rounded-sm bg-surface-subtle" />
          </span>
        </span>
      </span>
      <span className={cn("flex min-w-0 items-center gap-1 text-xs", active ? "text-ink" : "text-ink-soft")}>
        <span className="truncate">{label}</span>
        {active ? <Check aria-hidden className="size-3 shrink-0 text-accent" /> : null}
      </span>
    </button>
  );
}

/**
 * The same miniature at the size a settings row can afford.
 *
 * Drawn from the real tokens (`canvas` behind, `surface` card, `accent` dot and
 * pill), so it cannot drift from what the theme actually looks like. It reads
 * its colors from the `data-palette` its caller puts on the wrapper. Sized to
 * stay shorter than the label beside it, so the row keeps the height every other
 * setting has.
 */
function PaletteSwatch() {
  return (
    <span
      aria-hidden
      className="flex h-6 w-9 shrink-0 flex-col justify-between rounded border border-line bg-canvas p-1"
    >
      <span className="flex items-center gap-1">
        <span className="size-1 rounded-full bg-accent" />
        <span className="h-0.5 w-2.5 rounded-full bg-ink" />
      </span>
      <span className="flex items-center gap-0.5">
        <span className="h-1 w-3 rounded-sm bg-accent-fill" />
        <span className="h-1 w-1.5 rounded-sm bg-surface-subtle" />
      </span>
    </span>
  );
}
