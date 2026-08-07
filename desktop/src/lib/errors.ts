/**
 * Normalize an unknown caught value to a human-readable message. Replaces the
 * `error instanceof Error ? error.message : String(error)` incantation repeated
 * across catch blocks.
 */
export function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
