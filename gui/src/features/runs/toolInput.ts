import { isRecord } from "../../lib/objects";

/**
 * Robustly decode a tool-call input/output payload: agents sometimes
 * double-encode JSON (a JSON string whose value is itself a JSON string), so
 * parse up to three layers until a plain object surfaces.
 *
 * Lenient: a non-JSON string is returned unchanged (callers narrow with
 * `recordOf`); an empty/whitespace string becomes `null`.
 */
export function parseJsonish(value: unknown): unknown {
  let current = value;
  for (let index = 0; index < 3; index += 1) {
    if (isRecord(current))
      return current;
    if (typeof current !== "string")
      return current;
    const trimmed = current.trim();
    if (!trimmed)
      return null;
    try {
      current = JSON.parse(trimmed) as unknown;
    }
    catch {
      return current;
    }
  }
  return current;
}

/** Parse via `parseJsonish` and narrow to a plain object, else `null`. */
export function recordOf(value: unknown): Record<string, unknown> | null {
  const parsed = parseJsonish(value);
  return isRecord(parsed) ? parsed : null;
}

function keyList(keys: string | string[]): string[] {
  return Array.isArray(keys) ? keys : [keys];
}

/** First non-empty string value among `keys`, or `null`. */
export function stringField(record: Record<string, unknown> | null, keys: string | string[]): string | null {
  if (!record)
    return null;
  for (const key of keyList(keys)) {
    const field = record[key];
    if (typeof field === "string" && field.trim())
      return field;
  }
  return null;
}

/**
 * First non-empty string or finite number (stringified) among `keys`, or
 * `null`. Accepts numbers so fields like `exit_code: 0` are surfaced.
 */
export function numberOrStringField(record: Record<string, unknown> | null, keys: string | string[]): string | null {
  if (!record)
    return null;
  for (const key of keyList(keys)) {
    const field = record[key];
    if (typeof field === "string" && field.trim())
      return field;
    if (typeof field === "number" && Number.isFinite(field))
      return String(field);
  }
  return null;
}

/** Extract the `command` field from a (possibly double-encoded) tool input. */
export function toolCommand(input: string | null | undefined): string | null {
  return stringField(recordOf(input), "command");
}

/** Extract the target file path from a (possibly double-encoded) tool input. */
export function toolTarget(input: string | null | undefined): string | null {
  return stringField(recordOf(input), ["path", "filePath", "file_path", "targetPath", "target_path", "target"]);
}
