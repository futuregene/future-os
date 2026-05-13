/**
 * KeybindingManager — configurable key-to-action dispatch with conflict detection.
 * Ported from pi's keybindings.ts. Uses Symbol-based IDs for stable references
 * and supports user-level overrides via a flat config map.
 */

import type { KeyId } from "./keys.js";

export type KeyAction = () => boolean; // returns true if consumed
export type KeybindingContext = "global" | "editor" | "overlay" | "autocomplete";

/** Unique opaque ID returned when registering a binding. */
export type KeybindingId = symbol;

export interface KeybindingEntry {
  id: KeybindingId;
  key: KeyId;
  action: KeyAction;
  description: string;
  context?: KeybindingContext;
}

/**
 * User-level overrides: a flat map from key ID to the new action description.
 * Applied on top of programmatic bindings.  "key" → "" removes the binding entirely.
 */
export type UserOverrideMap = Record<string, string>; // key → description (or "" to unbind)

export class KeybindingManager {
  private bindings = new Map<string, KeybindingEntry[]>();
  private overrides: UserOverrideMap = {};

  /**
   * Register a keybinding.
   * Returns a unique Symbol-based ID that can be used to remove or update
   * this specific binding later.
   */
  add(key: KeyId, action: KeyAction, description: string, context?: KeybindingContext): KeybindingId {
    const id = Symbol(description);
    const entry: KeybindingEntry = { id, key, action, description, context };
    const existing = this.bindings.get(key);
    if (existing) {
      existing.push(entry);
    } else {
      this.bindings.set(key, [entry]);
    }
    return id;
  }

  /**
   * Remove a specific binding by its Symbol ID.
   * If id is omitted, removes all bindings for the key.
   */
  remove(key: KeyId, id?: KeybindingId): boolean {
    if (!id) {
      return this.bindings.delete(key);
    }
    const entries = this.bindings.get(key);
    if (!entries) return false;
    const idx = entries.findIndex((e) => e.id === id);
    if (idx === -1) return false;
    entries.splice(idx, 1);
    if (entries.length === 0) this.bindings.delete(key);
    return true;
  }

  /**
   * Apply user-level keybinding overrides.
   * Overrides are applied by description matching: if a user override config
   * maps "ctrl+p" → "Cycle model", we find the binding with that description
   * and keep it. When "ctrl+p" → "", we remove ALL bindings for that key.
   *
   * Use this to load from a config file (e.g. ~/.xihu/keybindings.json).
   */
  applyOverrides(overrides: UserOverrideMap): void {
    this.overrides = { ...overrides };
  }

  /**
   * Dispatch a key to registered bindings.
   * Runs all matching bindings in registration order. Stops on first that returns true.
   * Returns true if any binding consumed the key.
   */
  dispatch(key: string, context?: KeybindingContext): boolean {
    // Check user overrides first: if key is mapped to "", skip it entirely
    const overrideDesc = this.overrides[key] ?? null;
    if (overrideDesc === "") return false;

    const entries = this.bindings.get(key);
    if (!entries || entries.length === 0) return false;

    for (const entry of entries) {
      // If user has an override for this key, only fire the matching description
      if (overrideDesc !== null && overrideDesc !== undefined && entry.description !== overrideDesc) {
        continue;
      }
      if (context && entry.context && entry.context !== context) continue;
      try {
        if (entry.action()) return true;
      } catch {
        // Action threw — skip to next binding
      }
    }
    return false;
  }

  /** Return all registered entries (merged with overrides info). */
  getBindings(): KeybindingEntry[] {
    const all: KeybindingEntry[] = [];
    for (const entries of this.bindings.values()) {
      all.push(...entries);
    }
    return all;
  }

  /**
   * Return bindings that have more than one entry for the same key,
   * excluding those resolved by user overrides.
   */
  getConflicts(): { key: string; entries: KeybindingEntry[] }[] {
    const conflicts: { key: string; entries: KeybindingEntry[] }[] = [];
    for (const [key, entries] of this.bindings) {
      const overrideDesc = this.overrides[key];
      if (overrideDesc !== undefined) {
        if (overrideDesc === "") continue; // unbind — no conflict
        const resolved = entries.filter((e) => e.description === overrideDesc);
        if (resolved.length === 1) continue; // resolved by override
      }
      if (entries.length > 1) {
        conflicts.push({ key, entries });
      }
    }
    return conflicts;
  }

  /** Flattened map: key → descriptions (for help display). */
  getBindingMap(): Map<string, string[]> {
    const map = new Map<string, string[]>();
    for (const [key, entries] of this.bindings) {
      const overrideDesc = this.overrides[key];
      let visible = entries;
      if (overrideDesc !== undefined) {
        if (overrideDesc === "") continue;
        visible = entries.filter((e) => e.description === overrideDesc);
      }
      if (visible.length > 0) {
        map.set(key, visible.map((e) => e.description));
      }
    }
    return map;
  }

  /** Find a binding by its Symbol ID. */
  findById(id: KeybindingId): KeybindingEntry | undefined {
    for (const entries of this.bindings.values()) {
      const found = entries.find((e) => e.id === id);
      if (found) return found;
    }
    return undefined;
  }

  /** Get the user override map. */
  getOverrides(): UserOverrideMap {
    return { ...this.overrides };
  }

  /** Remove all bindings. */
  clear(): void {
    this.bindings.clear();
    this.overrides = {};
  }
}
