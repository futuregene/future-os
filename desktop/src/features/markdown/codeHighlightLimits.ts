/**
 * The largest source worth tokenizing: past this, `highlight` still works but
 * the result is not cached. That single bound is why full-screen previews gate
 * on it too — above it a 200KB source costs ~200ms of tokenizing plus tens of
 * thousands of spans, and the preview shows plain monospace instead.
 *
 * It lives in its own module so both the hook (which renders the fallback above
 * the bound) and the lazily loaded Shiki engine (which decides what to cache)
 * can read it without either pulling the other in: the engine is its own chunk,
 * and a value import from it would put Shiki back in the entry chunk.
 */
export const MAX_HIGHLIGHTABLE_CODE_LENGTH = 100_000;
