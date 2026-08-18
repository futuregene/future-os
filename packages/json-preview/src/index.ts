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

export function tokenizeJsonLine(line: string): JsonToken[] {
  const tokens: JsonToken[] = [];
  let plainStart = 0;
  let index = 0;

  while (index < line.length) {
    const char = line[index]!;
    const end = char === '"'
      ? jsonStringEnd(line, index)
      : numberEnd(line, index) ?? literalEnd(line, index);
    if (end === null) {
      index += 1;
      continue;
    }

    if (plainStart < index) tokens.push({ text: line.slice(plainStart, index), kind: "plain" });
    const text = line.slice(index, end);
    const kind: JsonToken["kind"] = char === '"'
      ? nextNonWhitespace(line, end) === ":"
        ? "key"
        : "string"
      : char === "t" || char === "f" || char === "n"
        ? "literal"
        : "number";
    tokens.push({ text, kind });
    index = end;
    plainStart = end;
  }
  if (plainStart < line.length) tokens.push({ text: line.slice(plainStart), kind: "plain" });
  return tokens;
}

/** Scans a JSON string in one pass, including malformed unterminated input. */
function jsonStringEnd(line: string, start: number): number | null {
  let escaped = false;
  for (let index = start + 1; index < line.length; index += 1) {
    const char = line[index]!;
    if (escaped) {
      escaped = false;
    } else if (char === "\\") {
      escaped = true;
    } else if (char === '"') {
      return index + 1;
    }
  }
  return null;
}

function numberEnd(line: string, start: number): number | null {
  let index = start;
  if (line[index] === "-") index += 1;

  if (line[index] === "0") {
    index += 1;
  } else if (isDigitOneToNine(line[index])) {
    index += 1;
    while (isDigit(line[index])) index += 1;
  } else {
    return null;
  }

  if (line[index] === "." && isDigit(line[index + 1])) {
    index += 2;
    while (isDigit(line[index])) index += 1;
  }
  if ((line[index] === "e" || line[index] === "E") && hasExponentDigits(line, index + 1)) {
    index += line[index + 1] === "+" || line[index + 1] === "-" ? 2 : 1;
    while (isDigit(line[index])) index += 1;
  }
  return index;
}

function literalEnd(line: string, start: number): number | null {
  for (const literal of ["true", "false", "null"]) {
    const end = start + literal.length;
    if (
      line.startsWith(literal, start)
      && !isWordCharacter(line[start - 1])
      && !isWordCharacter(line[end])
    ) return end;
  }
  return null;
}

function hasExponentDigits(line: string, start: number) {
  const first = line[start] === "+" || line[start] === "-" ? start + 1 : start;
  return isDigit(line[first]);
}

function isDigit(char: string | undefined) {
  return char !== undefined && char >= "0" && char <= "9";
}

function isDigitOneToNine(char: string | undefined) {
  return char !== undefined && char >= "1" && char <= "9";
}

function isWordCharacter(char: string | undefined) {
  return char !== undefined && ((char >= "a" && char <= "z") || (char >= "A" && char <= "Z") || isDigit(char) || char === "_");
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
