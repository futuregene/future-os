import { useTranslation } from "react-i18next";
import { cn } from "../../lib/cn";
import { PeerIconGlyph } from "./PeerIconGlyph";
import { PEER_ICONS } from "./peerIcons";

/**
 * The icon picker for a paired remote desktop.
 *
 * A row of glyphs rather than a text field, because the value it produces is
 * rendered in place of the machine's name in the conversation list — where it
 * must be same-width and distinguishable at a glance. Picking from a set is
 * what makes that true; it is also why the stored value is the icon *id*.
 *
 * The tiles are the phone's, value for value: 44px square (`layout.touchTarget`
 * there), a 12px radius (`radius.md`), a 20px glyph, an 8px gap, and the same
 * two states — a `line` border on `surface` for the rest, an `accent` border on
 * `accent-soft` for the machine's own. The two clients show the same machine's
 * mark and the same set to choose it from, so a picker that was visibly
 * smaller and squarer on the desktop read as a different control rather than
 * the same one on a larger screen. Keep the pairs in step: a change here is a
 * change in `mobile/src/features/settings/SettingsScreen.tsx` (`iconOption`).
 *
 * The glyph is a line icon from the shared set, not an emoji: a picture drawn
 * by the platform's emoji font would differ between macOS, Windows and the
 * phone, and the picker is where that was promised most loudly. Each tile draws
 * its icon in that icon's own colour, and the chosen one is marked by its frame
 * — a tile that recoloured its glyph would hide the very colour the choice is
 * about.
 */
export function PeerIconPicker({
  value,
  onChange,
}: {
  value: string | null;
  onChange: (icon: string) => void;
}) {
  const { t } = useTranslation("remotePeer");

  return (
    <div
      aria-label={t("icons.label")}
      className="flex flex-wrap items-center gap-2"
      role="radiogroup"
    >
      {PEER_ICONS.map((icon) => {
        const selected = value === icon.id;
        return (
          <button
            aria-checked={selected}
            aria-label={t(`icons.${icon.labelKey}`)}
            className={cn(
              "flex size-11 items-center justify-center rounded-xl border transition-colors",
              selected
                ? "border-accent bg-accent-soft"
                : "border-line bg-surface hover:bg-surface-subtle hover:border-line",
            )}
            key={icon.id}
            onClick={() => onChange(icon.id)}
            role="radio"
            title={t(`icons.${icon.labelKey}`)}
            type="button"
          >
            <PeerIconGlyph className="size-5" id={icon.id} />
          </button>
        );
      })}
    </div>
  );
}
