/**
 * Mount the app's shortcut dispatcher for the duration of a test.
 *
 * A view's command (the find bar's `⌘F`, the composer's focus key, the rail's
 * row walk) is only reachable through the one dispatcher the shell installs, so a
 * test of that view has to install it too. Otherwise the test would either press a
 * key nothing answers, or call the handler directly and prove nothing about the
 * path the app actually uses.
 *
 * Returns its own uninstaller, which also drops any command handlers left behind
 * by a test that failed before unmounting its view.
 */
import { act } from "react";
import { createRoot } from "react-dom/client";
import { clearCommandHandlers } from "../features/keyboard/commands";
import { ShortcutHost } from "../features/keyboard/ShortcutHost";

export function installShortcuts(): () => void {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  act(() => root.render(<ShortcutHost />));
  return () => {
    act(() => root.unmount());
    container.remove();
    clearCommandHandlers();
  };
}
