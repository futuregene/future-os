/**
 * The key that opens the palette, read live, for the dialog's own use.
 *
 * The palette cannot wait for the dispatcher to close it: while it is open it *is*
 * an overlay, and the dispatcher stands down behind one (`dispatch.ts` rule 4) —
 * which is what keeps `⌘K` from being a keystroke the dialog types into itself.
 * So the dialog asks the same table whether this press is its own key, using the
 * user's live binding: a palette rebound to `⌘⇧P` closes on `⌘⇧P`, not on `⌘K`.
 */
import type { BindingKeyEvent } from "../../lib/keybinding";
import { matchesBinding } from "../../lib/keybinding";
import { isMacOS } from "../../lib/platform";
import { resolvedBindingList } from "../keyboard/shortcuts";

export function isPaletteToggleKey(event: BindingKeyEvent, isMac: boolean = isMacOS): boolean {
  return resolvedBindingList("palette.open").some(binding => matchesBinding(binding, event, isMac));
}
