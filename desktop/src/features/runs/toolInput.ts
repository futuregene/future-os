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
  return firstStringField(input, "command");
}

/** Extract the target file path from a (possibly double-encoded) tool input. */
export function toolTarget(input: string | null | undefined): string | null {
  return firstStringField(input, [
    "path",
    "filePath",
    "file_path",
    "targetPath",
    "target_path",
    "target",
  ]);
}

/**
 * Field lookup that tolerates an incomplete JSON object: while a tool's
 * arguments are still streaming (`tool_delta` fragments folded into `input`),
 * `recordOf` can't parse the text yet, but a `"key": "value"` pair whose value
 * already closed its quote is safe to read off the raw text. Mirrors
 * thread-projection's `partialTargetFromArgsText` for the chat-area preview.
 */
function firstStringField(
  input: string | null | undefined,
  keys: string | string[],
): string | null {
  const parsed = parseJsonish(input);
  if (isRecord(parsed))
    return stringField(parsed, keys);
  // `parsed` is the innermost peeled string when the (double-encoded) input
  // couldn't be parsed all the way down — partial-match whichever text we have.
  const text
    = typeof parsed === "string" && parsed.trim()
      ? parsed
      : typeof input === "string" && input.trim()
        ? input
        : null;
  return text ? partialStringField(text, keys) : null;
}

/** First `"key": "<closed quoted value>"` pair present in `text`, or `null`. */
function partialStringField(
  text: string,
  keys: string | string[],
): string | null {
  for (const key of keyList(keys)) {
    const match = new RegExp(`"${key}"\\s*:\\s*("(?:[^"\\\\]|\\\\.)*")`).exec(text);
    if (!match?.[1])
      continue;
    try {
      const value = JSON.parse(match[1]) as unknown;
      if (typeof value === "string" && value.trim())
        return value;
    }
    catch {
      // Unescapable quote content — keep looking.
    }
  }
  return null;
}
