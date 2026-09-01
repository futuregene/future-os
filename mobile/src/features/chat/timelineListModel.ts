/**
 * Remote history stays chronological. The chat list alone consumes a reversed
 * copy so an older chronological prepend becomes an append in view space.
 * Existing visible item indices therefore remain stable while a page renders.
 */
export function newestFirst<T>(chronologicalItems: readonly T[]): T[] {
  return [...chronologicalItems].reverse();
}
