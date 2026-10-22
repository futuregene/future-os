import type { ThemePalette } from "../../lib/theme";
import { Check } from "lucide-react";
import { useTranslation } from "react-i18next";
import { cn } from "../../lib/cn";
import { setThemePalette, THEME_PALETTES, useThemePalette } from "../../lib/theme";

/**
 * The color-theme picker.
 *
 * A row of miniature windows rather than a `<Select>` of names, because a theme
 * name says nothing: "Nord" and "Gruvbox" are only distinguishable by looking at
 * them, and a user choosing a look is doing exactly that. This is how the apps
 * people compare us to do it (VS Code, Slack, Linear) — a preview you recognize
 * beats a list you have to try one by one.
 *
 * Each card carries its own `data-palette`, which is why the palette rules in
 * `styles/globals.css` match a nested preview element as well as the root: the
 * card paints itself in the palette it offers while the app keeps the one that
 * is actually active. The card inherits whichever rendition (`.dark`) the window
 * is in, so in a dark window every preview shows that palette's dark self.
 */
export function ThemePalettePicker() {
  const { t } = useTranslation("settings");
  const active = useThemePalette();

  return (
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
  );
}

/**
 * One palette: a tiny window in that palette plus its name.
 *
 * The preview is drawn from the real tokens (`canvas` behind, a `surface` card,
 * `ink` / `ink-muted` text bars and an `accent` dot), not from hard-coded
 * colors — so it cannot drift from what the theme actually looks like, and the
 * light/dark flip applies here the same way it does everywhere else.
 */
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
