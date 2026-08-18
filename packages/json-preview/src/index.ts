export const MAX_JSON_DEPTH = 128;
export const MAX_JSON_LINES = 50_000;
export const MAX_JSON_RICH_PREVIEW_BYTES = 1024 * 1024;
export const MAX_RAW_JSON_LINE_CHARS = 4096;

export interface JsonToken {
  text: string;
  kind: "plain" | "key" | "string" | "number" | "literal";
}

export interface FormattedJsonPreview {
  lines: string[];
  limited: boolean;
}

function nextNonWhitespace(source: string, from: number): string {
  for (let index = from; index < source.length; index += 1) {
    const char = source[index]!;
    if (!/\s/.test(char)) return char;
  }
  return "";
}

/**
 * Pretty-print JSON lexically instead of parsing and stringifying it. This
 * preserves large integer spellings, exponent notation, duplicate keys and
 * escape sequences exactly as received.
 */
export function formatJsonForPreview(source: string): FormattedJsonPreview {
  const lines: string[] = [];
  const stack: string[] = [];
  let current = "";
  let inString = false;
  let escaped = false;
  let limited = false;

  const pushLine = () => {
    if (lines.length >= MAX_JSON_LINES) {
      limited = true;
      return false;
    }
    lines.push(current);
    current = "  ".repeat(stack.length);
    return true;
  };

  for (let index = 0; index < source.length && !limited; index += 1) {
    const char = source[index]!;
    if (inString) {
      current += char;
      if (escaped) {
        escaped = false;
      } else if (char === "\\") {
        escaped = true;
      } else if (char === '"') {
        inString = false;
      }
      continue;
    }

    if (char === '"') {
      inString = true;
      current += char;
      continue;
    }
    if (/\s/.test(char)) continue;

    if (char === "{" || char === "[") {
      const closing = char === "{" ? "}" : "]";
      current += char;
      stack.push(closing);
      if (stack.length > MAX_JSON_DEPTH) {
        limited = true;
      } else if (nextNonWhitespace(source, index + 1) !== closing) {
        pushLine();
      }
      continue;
    }

    if (char === "}" || char === "]") {
      const expected = stack[stack.length - 1];
      if (expected === char) stack.pop();
      const trimmed = current.trimEnd();
      const isEmptyPair =
        (char === "}" && trimmed.endsWith("{")) || (char === "]" && trimmed.endsWith("["));
      if (!isEmptyPair && trimmed.trim().length > 0) {
        if (!pushLine()) break;
      }
      current = isEmptyPair ? trimmed + char : "  ".repeat(stack.length) + char;
      continue;
    }

    if (char === ",") {
      current += char;
      pushLine();
      continue;
    }
    if (char === ":") {
      current += ": ";
      continue;
    }
    current += char;
  }

  if (!limited && current.trim().length > 0) lines.push(current.trimEnd());
  if (lines.length === 0) lines.push("");
  return { lines, limited };
}

const JSON_TOKEN =
  /"(?:\\(?:["\\/bfnrt]|u[0-9a-fA-F]{4})|[^"\\])*"|-?(?:0|[1-9]\d*)(?:\.\d+)?(?:[eE][+-]?\d+)?|\b(?:true|false|null)\b/g;

export function tokenizeJsonLine(line: string): JsonToken[] {
  const tokens: JsonToken[] = [];
  let cursor = 0;
  for (const match of line.matchAll(JSON_TOKEN)) {
    const start = match.index;
    if (start > cursor) tokens.push({ text: line.slice(cursor, start), kind: "plain" });
    const text = match[0];
    const rest = line.slice(start + text.length).trimStart();
    const kind: JsonToken["kind"] = text.startsWith('"')
      ? rest.startsWith(":")
        ? "key"
        : "string"
      : text === "true" || text === "false" || text === "null"
        ? "literal"
        : "number";
    tokens.push({ text, kind });
    cursor = start + text.length;
  }
  if (cursor < line.length) tokens.push({ text: line.slice(cursor), kind: "plain" });
  return tokens;
}

export function rawJsonLines(source: string): FormattedJsonPreview {
  const lines: string[] = [];
  let limited = false;
  for (const sourceLine of source.split(/\r?\n/)) {
    // Malformed or minified JSON may be one multi-megabyte line. Chunk it so
    // one native/DOM text node cannot monopolize layout memory.
    for (let offset = 0; offset < Math.max(1, sourceLine.length); offset += MAX_RAW_JSON_LINE_CHARS) {
      if (lines.length >= MAX_JSON_LINES) {
        limited = true;
        break;
      }
      lines.push(sourceLine.slice(offset, offset + MAX_RAW_JSON_LINE_CHARS));
    }
    if (limited) break;
  }
  return { lines: lines.length > 0 ? lines : [""], limited };
}
