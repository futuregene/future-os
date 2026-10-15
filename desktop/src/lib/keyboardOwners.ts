import { useEffect } from "react";

/**
 * The surfaces that hold the keyboard **while they are up**: a modal, or a
 * floating menu / popover.
 *
 * A global handler for an unmodified key has to ask this before acting, because
 * a plain key has no modifier to make it obviously someone else's. The
 * transcript's ↑/↓ pair is the first such handler (see `questionNavKeys`): the
 * activity rail's menus move focus with the arrows, the terminal sends them to
 * the shell, and the composer's caret uses them — a jump underneath any of
 * those is the app fighting the user for one keystroke.
 *
 * Registration is by **presence, not focus**: WebKit does not focus a button on
 * click, so a menu opened from a toolbar leaves the caret wherever it was. A
 * focus test would answer "nobody owns the keyboard" while the menu is plainly
 * on screen and in use.
 *
 * This is deliberately a second, narrower registry next to
 * `components/ui/overlayStack`. That one decides which layer answers Escape and
 * whether the terminal's shortcut may fire; folding popovers into it would
 * change both of those, and neither question is this one.
 */
const owners: symbol[] = [];

/** True while a modal or a floating menu holds the keyboard. */
export function hasKeyboardOwner(): boolean {
  return owners.length > 0;
}

/**
 * Hold the keyboard while `active`. Registering a second owner while one is
 * open is fine and is the normal case for a menu inside a dialog: releasing one
 * leaves the other in place.
 */
export function useKeyboardOwner(active: boolean) {
  useEffect(() => {
    if (!active)
      return;
    const id = Symbol("keyboard-owner");
    owners.push(id);
    return () => {
      const index = owners.lastIndexOf(id);
      if (index !== -1)
        owners.splice(index, 1);
    };
  }, [active]);
}
