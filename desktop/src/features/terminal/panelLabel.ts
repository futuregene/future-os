/**
 * The panel toggle's accessible label, in one place.
 *
 * Two affordances collapse the same panel (the header button and the panel's own
 * ✕), and both must say the same thing — including the case where the user
 * removed the binding: "Hide terminal ()" reads as a bug, so the shortcut is only
 * mentioned when there is one.
 */
import type { TFunction } from "i18next";

export function terminalPanelLabel(
  t: TFunction<"terminal">,
  open: boolean,
  shortcut: string,
): string {
  if (shortcut)
    return t(open ? "collapse" : "expand", { shortcut });
  return t(open ? "collapseNoShortcut" : "expandNoShortcut");
}
