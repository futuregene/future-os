/**
 * Shortcut scopes: a keymap that is active only while a mode is on.
 *
 * The app already has one global keymap, and it is right for the things that are
 * always true — new conversation, find, panels, the terminal's toggle. A
 * **mode** needs the opposite property: while it is on, a key should mean what
 * the mode says, and it must be allowed to sit on a key the global keymap
 * already uses. The Vibe panel (full-screen grid of pinned conversations) is the
 * first such mode: in it `←`/`→` move between cells, `↩` zooms in, `1`–`9` jump,
 * `Esc` backs out — keys that would be unusable, or harmful, anywhere else.
 *
 * So a binding is matched **within a scope**. Resolution goes from the innermost
 * active scope outwards, and stops at the first command that matches, so:
 *
 * - a mode's `⌘[` shadows the global one while the mode is on, and the global
 *   meaning is back the moment it is off;
 * - two modes that never run together may share a key without a conflict;
 * - a key nothing in the active scopes binds is left alone (the dispatcher
 *   returns `null` and nothing is swallowed).
 *
 * **The innermost layer is the one where the user types** (`vibeInput`). It is
 * ranked last because it always sits on top while it is on — a cell's reading
 * layer may still be held underneath it — and it is the one layer whose own keys
 * must work *while the caret is in a text field* (`overEditable` in the catalog).
 * Everywhere else a bare key belongs to whatever has the caret.
 *
 * Registration is by presence, like `lib/keyboardOwners`: a scope is active while
 * a component says it is, which is what keeps "the mode is drawing a cell" and
 * "the mode owns the grid's arrow keys" from drifting apart.
 */
import { useEffect } from "react";

/** The scopes a command can be bound in, outermost first. */
export const SCOPES = ["global", "vibe", "vibeGrid", "vibeZoom", "vibeInput"] as const;
export type ShortcutScope = (typeof SCOPES)[number];

/**
 * Which scopes are active right now, **innermost first**.
 *
 * `global` is always present (it is not registered — it is the absence of a
 * mode). The others are pushed by `useShortcutScope` while their mode is on.
 */
const active: ShortcutScope[] = [];

/** The innermost-first list the dispatcher resolves against. */
export function activeScopes(): readonly ShortcutScope[] {
  // Ranked by SCOPES, not by registration order: `vibe` holds its scope from the
  // moment the panel opens while `vibeGrid` comes and goes as focus moves, so
  // "who was pushed last" would depend on effect ordering rather than on which
  // layer the user is actually in.
  const innermostFirst = SCOPES
    .filter(scope => scope !== "global" && active.includes(scope))
    .reverse();
  return [...innermostFirst, "global"];
}

/** Whether a scope owns the keyboard at all. */
export function isScopeActive(scope: ShortcutScope): boolean {
  return scope === "global" || active.includes(scope);
}

/**
 * Hold a scope while `on`.
 *
 * While it is held, the commands bound in it are matched before any global one.
 * Registering the same scope twice is fine (two cells both asking for the grid
 * layer is a bug elsewhere, but the release of one must not unlock the other).
 */
export function useShortcutScope(scope: Exclude<ShortcutScope, "global">, on: boolean): void {
  useEffect(() => {
    if (!on)
      return;
    active.push(scope);
    return () => {
      const index = active.lastIndexOf(scope);
      if (index !== -1)
        active.splice(index, 1);
    };
  }, [on, scope]);
}

/** Test seam: release every scope a failed test may have left behind. */
export function resetShortcutScopes(): void {
  active.length = 0;
}
