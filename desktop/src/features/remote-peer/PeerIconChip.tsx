import { cn } from "../../lib/cn";
import { PeerIconGlyph } from "./PeerIconGlyph";

/**
 * A paired machine's icon, on a chip.
 *
 * The box and the glyph inside it are role, not decoration: a machine's mark
 * shows up in a list row beside its name, in a page header beside its title,
 * and in the picker — and in all three it is the same square of surface. The
 * phone draws it the same way (`mobile/src/screens/DesktopsScreen.tsx`
 * `desktopIcon`, `SettingsScreen.tsx` `deviceIcon`): 36px, 12px radius,
 * `surface-subtle` behind, a 20px glyph. Same numbers here, so the same
 * machine is drawn the same on both clients.
 *
 *  - `row` — 36×44, for a machine's row in a list (the phone's list row).
 *  - `page` — 36×36, for a page header naming that machine (the phone's banner).
 *
 * The backing is `surface-subtle` in both. The phone's page variant sits on a
 * tinted banner; both desktop headers are plain `surface`, where a white chip
 * would disappear.
 */
export function PeerIconChip({
  id,
  variant = "row",
}: {
  id: string | null | undefined;
  variant?: "row" | "page";
}) {
  return (
    <span
      aria-hidden
      className={cn(
        "flex shrink-0 items-center justify-center rounded-xl bg-surface-subtle",
        variant === "row" ? "h-11 w-9" : "size-9",
      )}
    >
      <PeerIconGlyph className="size-5" id={id} />
    </span>
  );
}
