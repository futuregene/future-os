import { useTranslation } from "react-i18next";
import { cn } from "../../lib/cn";
import { PEER_ICONS } from "./peerIcons";

/**
 * The icon picker for a paired remote desktop.
 *
 * A row of glyphs rather than a text field, because the value it produces is
 * rendered in place of the machine's name in the conversation list — where it
 * must be same-width and distinguishable at a glance. Picking from a set is
 * what makes that true; it is also why the stored value is the icon *id*.
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
      className="flex flex-wrap items-center gap-1.5"
      role="radiogroup"
    >
      {PEER_ICONS.map((icon) => {
        const selected = value === icon.id;
        return (
          <button
            aria-checked={selected}
            aria-label={t(`icons.${icon.labelKey}`)}
            className={cn(
              "flex size-8 items-center justify-center rounded-md border text-base transition-colors",
              selected
                ? "border-accent bg-accent-soft text-accent"
                : "border-line-soft text-ink-soft hover:border-line hover:text-ink",
            )}
            key={icon.id}
            onClick={() => onChange(icon.id)}
            role="radio"
            title={t(`icons.${icon.labelKey}`)}
            type="button"
          >
            {icon.glyph}
          </button>
        );
      })}
    </div>
  );
}
