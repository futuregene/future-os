import type { KeyBinding } from "../../lib/keybinding";
/**
 * The user's keyboard bindings: where they are stored, what they resolve to, and
 * the subscriptions that keep the UI in step.
 *
 * **The storage is localStorage, and that is a deliberate, reversible choice.**
 * Bindings are per machine and per platform (rebinding `Ctrl+K` on Windows must
 * not follow the same account to a Mac, and means nothing to a phone), they are
 * read on a keystroke rather than on a data path, and every other preference of
 * that shape in this app — theme, font size, panel geometry — lives in
 * localStorage for exactly the same reasons. Nothing else reads this file's
 * storage directly: moving it to the backend's `app_settings` later means
 * changing `read()`/`write()` (and the two accessors at the bottom) here and
 * nowhere else.
 *
 * **Only differences are stored.** A command the user never touched has no entry,
 * so improving a default in `catalog.ts` reaches every user who did not override
 * it. `null` is an explicit "bound to nothing" and is stored.
 *
 * A stored value is untrusted input: it may come from an older version, a
 * hand-edited profile, or a newer one this build does not know. Unknown ids and
 * unparseable bindings are dropped on read, never guessed at.
 */
import type { CommandId, CommandSpec } from "./catalog";
import { useSyncExternalStore } from "react";
import { formatBinding, parseBinding, serializeBinding } from "../../lib/keybinding";
import { isMacOS } from "../../lib/platform";
import { COMMANDS, commandSpec, defaultBinding } from "./catalog";

const STORAGE_KEY = "future.shortcuts.v1";

/** Command id → binding text, or `null` for "the user removed it". */
export type ShortcutOverrides = Readonly<Record<string, string | null>>;

interface ShortcutsState {
  overrides: ShortcutOverrides;
}

const listeners = new Set<() => void>();

function sanitize(raw: unknown): ShortcutOverrides {
  if (typeof raw !== "object" || raw === null)
    return {};
  const source = (raw as { overrides?: unknown }).overrides;
  if (typeof source !== "object" || source === null)
    return {};
  const overrides: Record<string, string | null> = {};
  for (const [id, value] of Object.entries(source as Record<string, unknown>)) {
    if (!commandSpec(id))
      continue;
    if (value === null) {
      overrides[id] = null;
      continue;
    }
    if (typeof value !== "string")
      continue;
    const binding = parseBinding(value);
    // Storing the canonical text keeps `"Cmd+K"` and `"mod+k"` from being two
    // different settings for the same keys.
    if (binding)
      overrides[id] = serializeBinding(binding);
  }
  return overrides;
}

function read(): ShortcutOverrides {
  try {
    const stored = localStorage.getItem(STORAGE_KEY);
    if (!stored)
      return {};
    return sanitize(JSON.parse(stored));
  }
  catch {
    // Unreadable storage costs the user their customisations, not the app.
    return {};
  }
}

function write(overrides: ShortcutOverrides): void {
  try {
    if (Object.keys(overrides).length === 0)
      localStorage.removeItem(STORAGE_KEY);
    else
      localStorage.setItem(STORAGE_KEY, JSON.stringify({ version: 1, overrides }));
  }
  catch {
    // A full or disabled storage must not break the keyboard; the choice simply
    // does not survive a restart.
  }
}

let state: ShortcutsState = { overrides: read() };

function publish(overrides: ShortcutOverrides): void {
  state = { overrides };
  write(overrides);
  for (const listener of [...listeners])
    listener();
}

/** Subscribe to binding changes; returns the unsubscriber. */
export function subscribeShortcuts(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

function snapshot(): ShortcutsState {
  return state;
}

/** The current overrides, for code that must not re-render (dispatch paths). */
export function shortcutOverrides(): ShortcutOverrides {
  return state.overrides;
}

/** Subscribe a component to binding changes. */
export function useShortcutOverrides(): ShortcutOverrides {
  return useSyncExternalStore(subscribeShortcuts, snapshot, snapshot).overrides;
}

/** The text a command resolves to, or `null` when it is bound to nothing. */
export function resolvedBindingText(
  id: CommandId,
  overrides: ShortcutOverrides = state.overrides,
): string | null {
  if (id in overrides)
    return overrides[id] ?? null;
  return commandSpec(id)?.defaultBinding ?? null;
}

/** The parsed binding a command resolves to, or `null` when it is bound to nothing. */
export function resolvedBinding(
  id: CommandId,
  overrides: ShortcutOverrides = state.overrides,
): KeyBinding | null {
  const text = resolvedBindingText(id, overrides);
  return text ? parseBinding(text) : null;
}

/** Every command's live binding, keyed by id — what the dispatcher matches against. */
export function resolvedBindings(
  overrides: ShortcutOverrides = state.overrides,
): Map<CommandId, KeyBinding | null> {
  return new Map(COMMANDS.map(spec => [spec.id, resolvedBinding(spec.id, overrides)]));
}

/** The command a binding text would collide with, other than `except`. */
export function conflictingCommand(
  text: string,
  except: CommandId,
  overrides: ShortcutOverrides = state.overrides,
): CommandSpec | null {
  const candidate = parseBinding(text);
  if (!candidate)
    return null;
  const canonical = serializeBinding(candidate);
  for (const spec of COMMANDS) {
    if (spec.id === except)
      continue;
    const current = resolvedBindingText(spec.id, overrides);
    if (!current)
      continue;
    const parsed = parseBinding(current);
    if (parsed && serializeBinding(parsed) === canonical)
      return spec;
  }
  return null;
}

/**
 * Assign `text` to a command, or remove its binding with `null`.
 *
 * Assigning exactly the default drops the override rather than storing it, so a
 * command the user reset keeps following the default if it later improves.
 */
export function setCommandBinding(id: CommandId, text: string | null): void {
  const overrides = { ...state.overrides };
  const spec = commandSpec(id);
  const binding = text ? parseBinding(text) : null;
  const canonical = binding ? serializeBinding(binding) : null;
  if (canonical === null)
    overrides[id] = null;
  else if (spec && canonical === serializeBinding(defaultBinding(spec)))
    delete overrides[id];
  else
    overrides[id] = canonical;
  publish(overrides);
}

/** Forget a command's customisation. */
export function resetCommandBinding(id: CommandId): void {
  if (!(id in state.overrides))
    return;
  const overrides = { ...state.overrides };
  delete overrides[id];
  publish(overrides);
}

/** Forget every customisation. */
export function resetAllCommandBindings(): void {
  publish({});
}

/** A command's live binding in the platform's notation (`""` when unbound). */
export function bindingLabel(
  id: CommandId,
  overrides: ShortcutOverrides = state.overrides,
  isMac: boolean = isMacOS,
): string {
  const binding = resolvedBinding(id, overrides);
  return binding ? formatBinding(binding, isMac) : "";
}

/** `bindingLabel`, subscribed: a tooltip follows a rebinding without a reload. */
export function useBindingLabel(id: CommandId): string {
  const overrides = useShortcutOverrides();
  return bindingLabel(id, overrides);
}
