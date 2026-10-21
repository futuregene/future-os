import type { CommandId } from "../keyboard/catalog";
import { useEffect, useState } from "react";
import { isMacOS } from "../../lib/platform";
import { hasCommandHandler } from "../keyboard/commands";
import { bindingLabel, useShortcutOverrides } from "../keyboard/shortcuts";

/**
 * The panel's hint line, derived from the command table.
 *
 * Two rules, and both exist to keep the line from lying:
 *
 * - **Only a command with a handler is listed.** The registry answers "can this
 *   run right now"; a hint for a command nothing registered would promise a key
 *   that does nothing — exactly the failure a remote control cannot diagnose.
 * - **Only a command with a binding is listed.** A reader who removed a binding
 *   must not be told the key still works.
 *
 * The keys come from the live bindings, so a rebinding changes the hint on the
 * next render instead of after a restart. The label is the same
 * `shortcuts:commands.<id>` string the settings page shows, so the panel cannot
 * rename a command the settings page calls something else.
 */
export interface HintItem {
  id: CommandId;
  /** The keys as the platform writes them (`←`, `⌘⇧V`, `Menu`). */
  keys: string;
}

export function activeHints(
  ids: readonly CommandId[],
  deps: {
    hasHandler: (id: CommandId) => boolean;
    keysOf: (id: CommandId) => string;
  },
): HintItem[] {
  const items: HintItem[] = [];
  for (const id of ids) {
    if (!deps.hasHandler(id))
      continue;
    const keys = deps.keysOf(id);
    if (!keys)
      continue;
    items.push({ id, keys });
  }
  return items;
}

/**
 * `activeHints` against the live app: the registry, the bindings and the
 * platform's notation.
 *
 * Handlers are installed by effects, so the first render sees an empty registry.
 * The one-time flag below is what makes the second render (after every child's
 * effect has run) the one that draws the line — without it the hints would be
 * right every time except the first time, which is the only time they matter.
 */
export function useActiveHints(ids: readonly CommandId[]): HintItem[] {
  const overrides = useShortcutOverrides();
  const isMac = isMacOS;
  const [settled, setSettled] = useState(false);
  useEffect(() => {
    setSettled(true);
  }, []);
  if (!settled)
    return [];
  return activeHints(ids, {
    hasHandler: hasCommandHandler,
    keysOf: id => bindingLabel(id, overrides, isMac),
  });
}
