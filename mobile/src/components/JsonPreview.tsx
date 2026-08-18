import { useMemo } from "react";
import { FlatList, StyleSheet, Text, View } from "react-native";
import { colors, spacing } from "../theme/tokens";

const MAX_JSON_DEPTH = 128;
const MAX_JSON_LINES = 50_000;

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
 * escape sequences exactly as received from scientific tools.
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
      const expected = stack.at(-1);
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

function rawJsonLines(source: string): FormattedJsonPreview {
  const lines: string[] = [];
  let limited = false;
  for (const sourceLine of source.split(/\r?\n/)) {
    // A malformed JSON document may be one multi-megabyte line. Chunk it so a
    // single native Text view cannot monopolize layout memory.
    for (let offset = 0; offset < Math.max(1, sourceLine.length); offset += 4096) {
      if (lines.length >= MAX_JSON_LINES) {
        limited = true;
        break;
      }
      lines.push(sourceLine.slice(offset, offset + 4096));
    }
    if (limited) break;
  }
  return { lines: lines.length > 0 ? lines : [""], limited };
}

interface JsonPreviewProps {
  text: string;
  sourceTruncated: boolean;
  truncatedMessage: string;
  invalidMessage(detail: string): string;
  tooComplexMessage: string;
}

export function JsonPreview({
  text,
  sourceTruncated,
  truncatedMessage,
  invalidMessage,
  tooComplexMessage,
}: JsonPreviewProps) {
  const validationError = useMemo(() => {
    if (sourceTruncated) return null;
    try {
      JSON.parse(text);
      return null;
    } catch (error) {
      return error instanceof Error ? error.message : String(error);
    }
  }, [sourceTruncated, text]);
  const formatted = useMemo(
    () => (validationError ? rawJsonLines(text) : formatJsonForPreview(text)),
    [text, validationError],
  );

  return (
    <FlatList
      contentContainerStyle={styles.content}
      data={formatted.lines}
      initialNumToRender={40}
      keyExtractor={(_, index) => String(index)}
      ListHeaderComponent={
        sourceTruncated || validationError || formatted.limited ? (
          <View style={styles.notices}>
            {sourceTruncated ? <Text style={styles.notice}>{truncatedMessage}</Text> : null}
            {validationError ? (
              <Text style={styles.error}>{invalidMessage(validationError)}</Text>
            ) : null}
            {formatted.limited ? <Text style={styles.notice}>{tooComplexMessage}</Text> : null}
          </View>
        ) : null
      }
      maxToRenderPerBatch={60}
      removeClippedSubviews
      renderItem={({ item, index }) => (
        <View style={styles.row}>
          <Text style={styles.lineNumber}>{index + 1}</Text>
          <Text selectable style={styles.code}>
            {tokenizeJsonLine(item).map((token, tokenIndex) => (
              <Text key={tokenIndex} style={styles[token.kind]}>
                {token.text}
              </Text>
            ))}
          </Text>
        </View>
      )}
      updateCellsBatchingPeriod={25}
      windowSize={9}
    />
  );
}

const styles = StyleSheet.create({
  content: { padding: spacing.md, paddingBottom: spacing.xl },
  notices: { gap: spacing.sm, marginBottom: spacing.md },
  notice: { color: colors.inkMuted, fontSize: 13, lineHeight: 19 },
  error: { color: colors.danger, fontSize: 13, lineHeight: 19 },
  row: { flexDirection: "row", alignItems: "flex-start", minHeight: 21 },
  lineNumber: {
    width: 44,
    paddingRight: spacing.sm,
    color: colors.inkSoft,
    fontFamily: "monospace",
    fontSize: 12,
    lineHeight: 21,
    textAlign: "right",
  },
  code: { flex: 1, color: colors.ink, fontFamily: "monospace", fontSize: 13, lineHeight: 21 },
  plain: { color: colors.ink },
  key: { color: "#1d4ed8" },
  string: { color: "#047857" },
  number: { color: "#b45309" },
  literal: { color: "#7c3aed" },
});
