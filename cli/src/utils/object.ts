export function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

export function getRecord(value: unknown): Record<string, unknown> | undefined {
  return isRecord(value) ? value : undefined;
}

export function ensureRecordProperty(
  parent: Record<string, unknown>,
  key: string,
): Record<string, unknown> {
  const current = getRecord(parent[key]);
  if (current) {
    return current;
  }

  const next: Record<string, unknown> = {};
  parent[key] = next;
  return next;
}

export function isNodeError(error: unknown): error is NodeJS.ErrnoException {
  return error instanceof Error && "code" in error;
}
