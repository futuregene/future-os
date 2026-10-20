/**
 * Mounts the app's shortcut listener without mounting the app.
 *
 * `AppShell` calls `useShortcutDispatcher` directly. This component exists for the
 * tests of the views that own a command: a test of the find bar has to render the
 * dispatcher next to it, because the keystroke now travels dispatcher → command →
 * handler, and a test that skipped the dispatcher would be testing a path the app
 * no longer uses.
 */
import { useShortcutDispatcher } from "./useDispatcher";

export function ShortcutHost() {
  useShortcutDispatcher();
  return null;
}
