/**
 * Which component owns which command, right now.
 *
 * A command's handler is not a prop of the shell: "stop generating" lives in
 * `useAgentThreadState`, "find in conversation" in `ThreadSearch`, "previous
 * conversation" in the rail's own row order. Each of those registers its handler
 * while it is mounted, which also answers a second question the shell could not:
 * **is the command runnable at all**? No conversation open means nobody
 * registered `thread.find`, and the dispatcher then leaves `⌘F` alone instead of
 * swallowing it for a no-op.
 *
 * The registry is module-level rather than React state because a keystroke must
 * not be able to observe a half-committed registration, and because the same
 * entry point (`runCommand`) serves the dispatcher, the macOS menu and the
 * command palette.
 */
import type { CommandId } from "./catalog";
import { useEffect } from "react";
import { useCommittedRef } from "../../lib/useCommittedRef";

interface Registration {
  run: () => void;
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
export function registerCommand(id: CommandId, run: () => void): () => void {
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

/** Run a command now. Returns false when nothing was registered for it. */
export function runCommand(id: CommandId): boolean {
  const registration = handlers.get(id);
  if (!registration)
    return false;
  registration.run();
  return true;
}

/**
 * Register a handler for as long as this component is mounted and `enabled`.
 *
 * The handler is read through a committed ref, so re-rendering a component with
 * a new closure does not re-register (and does not reopen the window in which the
 * command has no owner).
 */
export function useCommand(id: CommandId, run: () => void, enabled = true): void {
  const handlerRef = useCommittedRef(run);
  useEffect(() => {
    if (!enabled)
      return;
    return registerCommand(id, () => handlerRef.current());
  }, [enabled, handlerRef, id]);
}

/** Test seam: drop every registration (a test mounts components in isolation). */
export function clearCommandHandlers(): void {
  handlers.clear();
}
