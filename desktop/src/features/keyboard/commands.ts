import type { KeyBinding } from "../../lib/keybinding";
/**
 * Who owns a command, and how one runs.
 *
 * The registry itself is unchanged by scopes: a command has one handler, and the
 * view* that owns it is the one mounted. What scopes add is that the same key can
 * mean different commands in different modes, and the mode decides which of them
 * is listening — the rail owns `thread.next` outside the panel, the panel owns it
 * inside, and they are never both mounted.
 */
import type { CommandId, ShortcutScope } from "./catalog";
import { useEffect } from "react";
import { useCommittedRef } from "../../lib/useCommittedRef";

interface Registration {
  run: (binding: KeyBinding | undefined) => void;
}

/**
 * One handler per command.
 *
 * A later registration replaces an earlier one (two mounted instances of the
 * same view are a bug elsewhere, and the newest one is the one on screen), and
 * unregistering restores nothing: the component that replaced it owns it until
 * it unmounts itself.
 */
const handlers = new Map<CommandId, Registration>();

/** Register a handler; the returned function unregisters this one only. */
export function registerCommand(
  id: CommandId,
  run: (binding: KeyBinding | undefined) => void,
): () => void {
  const registration: Registration = { run };
  handlers.set(id, registration);
  return () => {
    if (handlers.get(id) === registration)
      handlers.delete(id);
  };
}

/** Whether a command can run at all (someone has registered it). */
export function hasCommandHandler(id: CommandId): boolean {
  return handlers.has(id);
}

/**
 * Run a command now.
 *
 * `binding` is the key that asked for it, for the commands that answer to more
 * than one (`vibe.cell` picks the cell from the digit). Returns false when
 * nothing was registered for it.
 */
export function runCommand(id: CommandId, binding?: KeyBinding): boolean {
  const registration = handlers.get(id);
  if (!registration)
    return false;
  registration.run(binding);
  return true;
}

/**
 * Register a handler for as long as this component is mounted and `enabled`.
 *
 * The handler is read through a committed ref, so re-rendering a component with
 * a new closure does not re-register (and does not reopen the window in which the
 * command has no owner).
 *
 * The scope this command belongs to is *not* taken here — it is the table's
 * (`catalog.ts`), and `useShortcutScope` is what turns a mode's keymap on. A
 * command whose scope is off is simply never matched, so a mounted handler with a
 * dormant scope is inert rather than wrong.
 */
export function useCommand(
  id: CommandId,
  run: (binding?: KeyBinding) => void,
  enabled = true,
): void {
  const handlerRef = useCommittedRef(run);
  useEffect(() => {
    if (!enabled)
      return;
    return registerCommand(id, binding => handlerRef.current(binding));
  }, [enabled, handlerRef, id]);
}

/** Test seam: drop every registration (a test mounts components in isolation). */
export function clearCommandHandlers(): void {
  handlers.clear();
}

/**
 * The scope a command belongs to, for the settings page's grouping.
 *
 * Re-exported so a view that composes a hint ("while the panel is open") does not
 * have to reach into the catalog directly.
 */
export type { ShortcutScope };
