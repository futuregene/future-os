/**
 * The window listener that runs commands — installed once, by `AppShell`.
 *
 * `dispatch.ts` holds the rules; this file holds the one thing the rules cannot
 * own: which part of the app the event came from, and the fact that a handled key
 * must not also reach the page.
 *
 * It listens in the **capture** phase so the terminal's xterm instance cannot
 * swallow a command it does not own first (xterm attaches its handler to its own
 * textarea, and it fires before the bubble-phase listeners on `window`). A key it
 * does handle is stopped there: `preventDefault` keeps the browser's own meaning
 * away, `stopPropagation` keeps the focused view from acting on it a second time.
 */
import { useEffect } from "react";
import { hasOpenOverlay } from "../../components/ui/overlayStack";
import { hasKeyboardOwner } from "../../lib/keyboardOwners";
import { isMacOS } from "../../lib/platform";
import { hasCommandHandler, runCommand } from "./commands";
import { dispatchShortcut, TERMINAL_SURFACE_SELECTOR } from "./dispatch";
import { resolvedBindings, useShortcutOverrides } from "./shortcuts";

function eventInTerminal(target: EventTarget | null): boolean {
  return target instanceof Element && target.closest(TERMINAL_SURFACE_SELECTOR) !== null;
}

/**
 * Install the global shortcut listener for this window.
 *
 * The resolved bindings are recomputed when the user changes one, which is what
 * makes a rebinding take effect on the next key press instead of at the next
 * launch.
 */
export function useShortcutDispatcher(): void {
  const overrides = useShortcutOverrides();
  useEffect(() => {
    const bindings = resolvedBindings(overrides);
    const onKeyDown = (event: KeyboardEvent) => {
      const id = dispatchShortcut(event, {
        isMac: isMacOS,
        bindings,
        hasHandler: hasCommandHandler,
        hasOpenOverlay,
        hasKeyboardOwner,
        inTerminal: eventInTerminal(event.target),
      });
      if (!id)
        return;
      // Ours now: stop the page's own behaviour (find-in-page, and whatever the
      // focused view would have done with the key) and stop the event reaching
      // the views — a shortcut the app ran must not also act on whatever was
      // focused when it was pressed.
      event.preventDefault();
      event.stopPropagation();
      runCommand(id);
    };
    window.addEventListener("keydown", onKeyDown, true);
    return () => window.removeEventListener("keydown", onKeyDown, true);
  }, [overrides]);
}
