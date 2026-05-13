/**
 * KeybindingManager — configurable key-to-action dispatch with conflict detection.
 * Ported from pi's keybindings.ts.
 */

import type { KeyId } from "./keys.js";

export type KeyAction = () => boolean; // returns true if consumed

export type KeybindingContext = "global" | "editor" | "overlay" | "autocomplete";

export interface KeybindingEntry {
  key: KeyId;
  action: KeyAction;
  description: string;
  context?: KeybindingContext;
}

export class KeybindingManager {
  private bindings = new Map<string, KeybindingEntry[]>();

  /** Register a keybinding. Returns an unbind function. */
  add(key: KeyId, action: KeyAction, description: string, context?: KeybindingContext): () => void {
    const entry: KeybindingEntry = { key, action, description, context };
    const existing = this.bindings.get(key);
    if (existing) {
      existing.push(entry);
    } else {
      this.bindings.set(key, [entry]);
    }
    return () => this.remove(key, action);
  }

  /** Remove a keybinding. If action is omitted, removes all bindings for the key. */
  remove(key: KeyId, action?: KeyAction): void {
    if (!action) {
      this.bindings.delete(key);
      return;
    }
    const entries = this.bindings.get(key);
    if (!entries) return;
    const idx = entries.findIndex((e) => e.action === action);
    if (idx !== -1) entries.splice(idx, 1);
    if (entries.length === 0) this.bindings.delete(key);
  }

  /**
   * Dispatch a key to registered bindings.
   * Runs all matching bindings in registration order. Stops on first that returns true.
   * Returns true if any binding consumed the key.
   */
  dispatch(key: string, context?: KeybindingContext): boolean {
    const entries = this.bindings.get(key);
    if (!entries || entries.length === 0) return false;

    for (const entry of entries) {
      if (context && entry.context && entry.context !== context) continue;
      try {
        if (entry.action()) return true;
      } catch {
        // Action threw — skip to next binding
      }
    }
    return false;
  }

  /** Return bindings that have more than one entry for the same key. */
  getConflicts(): { key: string; entries: KeybindingEntry[] }[] {
    const conflicts: { key: string; entries: KeybindingEntry[] }[] = [];
    for (const [key, entries] of this.bindings) {
      if (entries.length > 1) {
        conflicts.push({ key, entries });
      }
    }
    return conflicts;
  }

  /** Return all registered bindings. */
  getBindings(): KeybindingEntry[] {
    const all: KeybindingEntry[] = [];
    for (const entries of this.bindings.values()) {
      all.push(...entries);
    }
    return all;
  }

  /** Flattened map: key → descriptions (for help display). */
  getBindingMap(): Map<string, string[]> {
    const map = new Map<string, string[]>();
    for (const [key, entries] of this.bindings) {
      map.set(key, entries.map((e) => e.description));
    }
    return map;
  }

  /** Remove all bindings. */
  clear(): void {
    this.bindings.clear();
  }
}
