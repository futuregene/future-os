/**
 * KeybindingManager — configurable key-to-action dispatch with conflict detection.
 * Ported from pi's keybindings.ts. Uses Symbol-based IDs for stable references
 * and supports user-level overrides via a flat config map.
 */
export class KeybindingManager {
    bindings = new Map();
    overrides = {};
    /**
     * Register a keybinding.
     * Returns a unique Symbol-based ID that can be used to remove or update
     * this specific binding later.
     */
    add(key, action, description, context) {
        const id = Symbol(description);
        const entry = { id, key, action, description, context };
        const existing = this.bindings.get(key);
        if (existing) {
            existing.push(entry);
        }
        else {
            this.bindings.set(key, [entry]);
        }
        return id;
    }
    /**
     * Remove a specific binding by its Symbol ID.
     * If id is omitted, removes all bindings for the key.
     */
    remove(key, id) {
        if (!id) {
            return this.bindings.delete(key);
        }
        const entries = this.bindings.get(key);
        if (!entries)
            return false;
        const idx = entries.findIndex((e) => e.id === id);
        if (idx === -1)
            return false;
        entries.splice(idx, 1);
        if (entries.length === 0)
            this.bindings.delete(key);
        return true;
    }
    /**
     * Apply user-level keybinding overrides.
     * Overrides are applied by description matching: if a user override config
     * maps "ctrl+p" → "Cycle model", we find the binding with that description
     * and keep it. When "ctrl+p" → "", we remove ALL bindings for that key.
     *
     * Use this to load from a config file (e.g. ~/.xihu_tui/keybindings.json).
     */
    applyOverrides(overrides) {
        this.overrides = { ...overrides };
    }
    /**
     * Dispatch a key to registered bindings.
     * Runs all matching bindings in registration order. Stops on first that returns true.
     * Returns true if any binding consumed the key.
     */
    dispatch(key, context) {
        // Check user overrides first: if key is mapped to "", skip it entirely
        const overrideDesc = this.overrides[key] ?? null;
        if (overrideDesc === "")
            return false;
        const entries = this.bindings.get(key);
        if (!entries || entries.length === 0)
            return false;
        for (const entry of entries) {
            // If user has an override for this key, only fire the matching description
            if (overrideDesc !== null && overrideDesc !== undefined && entry.description !== overrideDesc) {
                continue;
            }
            if (context && entry.context && entry.context !== context)
                continue;
            try {
                if (entry.action())
                    return true;
            }
            catch {
                // Action threw — skip to next binding
            }
        }
        return false;
    }
    /** Return all registered entries (merged with overrides info). */
    getBindings() {
        const all = [];
        for (const entries of this.bindings.values()) {
            all.push(...entries);
        }
        return all;
    }
    /**
     * Return bindings that have more than one entry for the same key,
     * excluding those resolved by user overrides.
     */
    getConflicts() {
        const conflicts = [];
        for (const [key, entries] of this.bindings) {
            const overrideDesc = this.overrides[key];
            if (overrideDesc !== undefined) {
                if (overrideDesc === "")
                    continue; // unbind — no conflict
                const resolved = entries.filter((e) => e.description === overrideDesc);
                if (resolved.length === 1)
                    continue; // resolved by override
            }
            if (entries.length > 1) {
                conflicts.push({ key, entries });
            }
        }
        return conflicts;
    }
    /** Flattened map: key → descriptions (for help display). */
    getBindingMap() {
        const map = new Map();
        for (const [key, entries] of this.bindings) {
            const overrideDesc = this.overrides[key];
            let visible = entries;
            if (overrideDesc !== undefined) {
                if (overrideDesc === "")
                    continue;
                visible = entries.filter((e) => e.description === overrideDesc);
            }
            if (visible.length > 0) {
                map.set(key, visible.map((e) => e.description));
            }
        }
        return map;
    }
    /** Find a binding by its Symbol ID. */
    findById(id) {
        for (const entries of this.bindings.values()) {
            const found = entries.find((e) => e.id === id);
            if (found)
                return found;
        }
        return undefined;
    }
    /** Get the user override map. */
    getOverrides() {
        return { ...this.overrides };
    }
    /** Remove all bindings. */
    clear() {
        this.bindings.clear();
        this.overrides = {};
    }
}
