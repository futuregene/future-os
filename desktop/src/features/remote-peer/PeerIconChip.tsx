import type { ReactNode } from "react";
import { cn } from "../../lib/cn";

/**
 * A paired machine's icon, drawn the way the phone draws it.
 *
 * The phone puts the machine's glyph on a soft rounded chip — a 36px box, 12px
 * radius, `surfaceSubtle` behind, a 20px glyph inside — in its deskop list
 * (`mobile/src/screens/DesktopsScreen.tsx`, `desktopIcon`) and on a machine's
 * settings page (`deviceIcon`). The desktop drew the same machine as a bare
 * 12–18px emoji instead, so the one mark a user picks to recognise a machine
 * looked smaller, unbacked and washed out here than on the phone — two designs
 * for one thing.
 *
 * So the glyph keeps the phone's box and its size on the desktop too:
 *
 *  - `row` — 36×44, for a machine's row in a list (same as the phone's list row).
 *  - `page` — 36×36, for a page header naming that machine (same as the phone's
 *    machine banner).
 *
 * The backing is `surface-subtle` in both, which is what the phone's list row
 * uses; its page variant sits on a tinted banner, and both desktop headers are
 * plain `surface`, where a white chip would be invisible.
 */
export function PeerIconChip({
  glyph,
  variant = "row",
}: {
  glyph: ReactNode;
  variant?: "row" | "page";
}) {
  return (
    <span
      aria-hidden
      className={cn(
        "flex shrink-0 items-center justify-center rounded-xl bg-surface-subtle text-xl leading-none",
        variant === "row" ? "h-11 w-9" : "size-9",
      )}
    >
      {glyph}
    </span>
  );
}
