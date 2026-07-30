import { Linking, StyleSheet, Text, View } from "react-native";
import { colors, radius, spacing } from "../theme/tokens";

interface MarkdownTextProps {
  text: string;
}

type Align = "left" | "center" | "right" | null;

type ListItem = { text: string; checked: boolean | null };

type Block =
  | { kind: "paragraph"; text: string }
  | { kind: "heading"; level: number; text: string }
  | { kind: "code"; text: string }
  | { kind: "list"; ordered: boolean; items: ListItem[] }
  | { kind: "rule" }
  | { kind: "quote"; text: string }
  | { kind: "table"; headers: string[]; aligns: Align[]; rows: string[][] };

// Placeholder used while splitting table cells so an escaped pipe (`\|`) inside a
// cell is not treated as a column separator. Picked to never occur in real text.
const CELL_PIPE = "§PIPE§";

const TABLE_SEP_RE = /^\s*\|?\s*:?-{1,}:?\s*(\|\s*:?-{1,}:?\s*)*\|?\s*$/;
const QUOTE_RE = /^\s*>\s?(.*)$/;
const HEADING_RE = /^\s{0,3}(#{1,6})\s+(.+)$/;
const LIST_RE = /^\s*([-*+]|\d+\.)\s+(.+)$/;
const RULE_RE = /^\s{0,3}([-*_])\1\1+\s*$/;
const TASK_RE = /^\[([ xX])\]\s+(.*)$/;

function isTableSep(line: string): boolean {
  return line.includes("|") && TABLE_SEP_RE.test(line);
}

function splitCells(line: string): string[] {
  let s = line.trim().replace(/\\\|/g, CELL_PIPE);
  if (s.startsWith("|")) s = s.slice(1);
  if (s.endsWith("|")) s = s.slice(0, -1);
  return s.split("|").map(cell => cell.split(CELL_PIPE).join("|").trim());
}

function parseAligns(sepLine: string, columnCount: number): Align[] {
  const cells = splitCells(sepLine);
  const aligns: Align[] = [];
  for (let i = 0; i < columnCount; i += 1) {
    const cell = (cells[i] ?? "").trim();
    if (/^:-+:$/.test(cell)) aligns.push("center");
    else if (/^:-+$/.test(cell)) aligns.push("left");
    else if (/^-+:$/.test(cell)) aligns.push("right");
    else aligns.push(null);
  }
  return aligns;
}

// GFM normalises every row to the header's column count: extra cells are dropped,
// missing cells are padded with empty strings (mirrors the GUI's tableToFutureNode).
function normalizeRow(cells: string[], columnCount: number): string[] {
  if (cells.length >= columnCount) return cells.slice(0, columnCount);
  return [...cells, ...Array.from({ length: columnCount - cells.length }, () => "")];
}

// Used inside the paragraph-merge loop so a table/quote/etc. that follows a
// paragraph without a blank line still breaks the paragraph out correctly.
function isBlockStart(line: string, lineAfter: string | undefined): boolean {
  if (line.trimStart().startsWith("```")) return true;
  if (RULE_RE.test(line)) return true;
  if (HEADING_RE.test(line)) return true;
  if (LIST_RE.test(line)) return true;
  if (QUOTE_RE.test(line)) return true;
  if (line.includes("|") && lineAfter !== undefined && isTableSep(lineAfter)) return true;
  return false;
}

export function blocksFromMarkdown(text: string): Block[] {
  const lines = text.replace(/\r\n/g, "\n").split("\n");
  const blocks: Block[] = [];
  let index = 0;

  while (index < lines.length) {
    const line = lines[index] ?? "";
    if (!line.trim()) {
      index += 1;
      continue;
    }
    if (line.trimStart().startsWith("```")) {
      const code: string[] = [];
      index += 1;
      while (index < lines.length && !(lines[index] ?? "").trimStart().startsWith("```")) {
        code.push(lines[index] ?? "");
        index += 1;
      }
      if (index < lines.length) index += 1;
      blocks.push({ kind: "code", text: code.join("\n") });
      continue;
    }
    if (RULE_RE.test(line)) {
      blocks.push({ kind: "rule" });
      index += 1;
      continue;
    }
    const heading = line.match(HEADING_RE);
    if (heading) {
      blocks.push({ kind: "heading", level: heading[1]?.length ?? 1, text: heading[2] ?? "" });
      index += 1;
      continue;
    }
    const list = line.match(LIST_RE);
    if (list) {
      const ordered = /\d+\./.test(list[1] ?? "");
      const items: ListItem[] = [];
      while (index < lines.length) {
        const item = (lines[index] ?? "").match(LIST_RE);
        if (!item || /\d+\./.test(item[1] ?? "") !== ordered) break;
        const content = item[2] ?? "";
        if (!ordered) {
          const task = content.match(TASK_RE);
          if (task) {
            items.push({ text: task[2] ?? "", checked: (task[1] ?? "").toLowerCase() === "x" });
            index += 1;
            continue;
          }
        }
        items.push({ text: content, checked: null });
        index += 1;
      }
      blocks.push({ kind: "list", ordered, items });
      continue;
    }
    if (line.includes("|") && isTableSep(lines[index + 1] ?? "")) {
      const headers = splitCells(line);
      const columnCount = headers.length;
      const aligns = parseAligns(lines[index + 1] ?? "", columnCount);
      index += 2;
      const rows: string[][] = [];
      while (index < lines.length) {
        const row = lines[index] ?? "";
        if (!row.trim() || !row.includes("|")) break;
        rows.push(normalizeRow(splitCells(row), columnCount));
        index += 1;
      }
      blocks.push({ kind: "table", headers: normalizeRow(headers, columnCount), aligns, rows });
      continue;
    }
    const quote = line.match(QUOTE_RE);
    if (quote) {
      const inner: string[] = [quote[1] ?? ""];
      index += 1;
      while (index < lines.length) {
        const next = (lines[index] ?? "").match(QUOTE_RE);
        if (!next) break;
        inner.push(next[1] ?? "");
        index += 1;
      }
      blocks.push({ kind: "quote", text: inner.join("\n") });
      continue;
    }
    const paragraph = [line];
    index += 1;
    while (index < lines.length && (lines[index] ?? "").trim()) {
      const nextLine = lines[index] ?? "";
      if (isBlockStart(nextLine, lines[index + 1])) break;
      paragraph.push(nextLine);
      index += 1;
    }
    blocks.push({ kind: "paragraph", text: paragraph.join("\n") });
  }
  return blocks;
}

function InlineMarkdown({ text }: { text: string }) {
  const parts = text.split(/(\*\*[^*]+\*\*|~~[^~]+~~|`[^`]+`|\*[^*]+\*|\[[^\]]+\]\([^\s)]+\))/g);
  return (
    <>
      {parts.map((part, index) => {
        if (/^\*\*[^*]+\*\*$/.test(part)) {
          return (
            <Text key={index} style={styles.bold}>
              {part.slice(2, -2)}
            </Text>
          );
        }
        if (/^~~[^~]+~~$/.test(part)) {
          return (
            <Text key={index} style={styles.strike}>
              {part.slice(2, -2)}
            </Text>
          );
        }
        if (/^`[^`]+`$/.test(part)) {
          return (
            <Text key={index} style={styles.inlineCode}>
              {part.slice(1, -1)}
            </Text>
          );
        }
        if (/^\*[^*]+\*$/.test(part)) {
          return (
            <Text key={index} style={styles.italic}>
              {part.slice(1, -1)}
            </Text>
          );
        }
        const link = part.match(/^\[([^\]]+)\]\(([^)\s]+)\)$/);
        if (link) {
          return (
            <Text
              key={index}
              onPress={() => void Linking.openURL(link[2] ?? "")}
              style={styles.link}
            >
              {link[1] ?? ""}
            </Text>
          );
        }
        return part;
      })}
    </>
  );
}

function renderBlock(block: Block, index: number) {
  if (block.kind === "rule") return <View key={index} style={styles.rule} />;
  if (block.kind === "code") {
    return (
      <Text key={index} selectable style={styles.code}>
        {block.text}
      </Text>
    );
  }
  if (block.kind === "heading") {
    return (
      <Text
        key={index}
        selectable
        style={[styles.heading, block.level <= 2 ? styles.headingLarge : null]}
      >
        <InlineMarkdown text={block.text} />
      </Text>
    );
  }
  if (block.kind === "quote") {
    return (
      <View key={index} style={styles.quote}>
        {blocksFromMarkdown(block.text).map((child, childIndex) => renderBlock(child, childIndex))}
      </View>
    );
  }
  if (block.kind === "table") {
    return (
      <View key={index} style={styles.table}>
        <View style={[styles.tableRow, styles.tableHead]}>
          {block.headers.map((header, columnIndex) => (
            <Text
              key={columnIndex}
              selectable
              style={[
                styles.th,
                columnIndex > 0 ? styles.cellBorderLeft : null,
                { textAlign: block.aligns[columnIndex] ?? "left" },
              ]}
            >
              <InlineMarkdown text={header} />
            </Text>
          ))}
        </View>
        {block.rows.map((row, rowIndex) => (
          <View
            key={rowIndex}
            style={[
              styles.tableRow,
              styles.tableBodyRow,
              rowIndex % 2 === 1 ? styles.tableRowZebra : null,
            ]}
          >
            {row.map((cell, columnIndex) => (
              <Text
                key={columnIndex}
                selectable
                style={[
                  styles.td,
                  columnIndex > 0 ? styles.cellBorderLeft : null,
                  { textAlign: block.aligns[columnIndex] ?? "left" },
                ]}
              >
                <InlineMarkdown text={cell} />
              </Text>
            ))}
          </View>
        ))}
      </View>
    );
  }
  if (block.kind === "list") {
    return (
      <View key={index} style={styles.list}>
        {block.items.map((item, itemIndex) => (
          <View key={itemIndex} style={styles.listRow}>
            {item.checked === null ? (
              <Text style={styles.listBullet}>{block.ordered ? `${itemIndex + 1}.` : "•"}</Text>
            ) : (
              <View style={[styles.checkbox, item.checked ? styles.checkboxChecked : null]}>
                {item.checked ? <Text style={styles.checkMark}>✓</Text> : null}
              </View>
            )}
            <Text selectable style={styles.listItemText}>
              <InlineMarkdown text={item.text} />
            </Text>
          </View>
        ))}
      </View>
    );
  }
  return (
    <Text key={index} selectable style={styles.paragraph}>
      <InlineMarkdown text={block.text} />
    </Text>
  );
}

export function MarkdownText({ text }: MarkdownTextProps) {
  return <View>{blocksFromMarkdown(text).map((block, index) => renderBlock(block, index))}</View>;
}

const styles = StyleSheet.create({
  paragraph: { color: colors.ink, fontSize: 17, lineHeight: 26, marginBottom: spacing.md },
  heading: {
    color: colors.inkStrong,
    fontSize: 19,
    fontWeight: "700",
    lineHeight: 26,
    marginBottom: spacing.sm,
  },
  headingLarge: { fontSize: 23, lineHeight: 30 },
  bold: { fontWeight: "700" },
  italic: { fontStyle: "italic" },
  strike: { textDecorationLine: "line-through" },
  inlineCode: {
    color: colors.inkStrong,
    backgroundColor: colors.surfaceSubtle,
    fontFamily: "monospace",
  },
  link: { color: colors.accent, textDecorationLine: "underline" },
  rule: { height: 1, marginVertical: spacing.md, backgroundColor: colors.line },
  code: {
    marginBottom: spacing.md,
    padding: spacing.md,
    color: colors.ink,
    borderRadius: radius.md,
    backgroundColor: colors.surfaceSubtle,
    fontFamily: "monospace",
    fontSize: 13,
    lineHeight: 20,
  },
  quote: {
    marginBottom: spacing.md,
    paddingLeft: spacing.md,
    borderLeftWidth: 3,
    borderLeftColor: colors.accent,
  },
  list: { marginBottom: spacing.md },
  listRow: { flexDirection: "row", alignItems: "flex-start", marginBottom: spacing.xs },
  listBullet: {
    width: 22,
    marginRight: spacing.xs,
    color: colors.ink,
    fontSize: 17,
    lineHeight: 26,
  },
  listItemText: { flex: 1, color: colors.ink, fontSize: 17, lineHeight: 26 },
  checkbox: {
    width: 18,
    height: 18,
    marginTop: 4,
    marginRight: spacing.sm,
    alignItems: "center",
    justifyContent: "center",
    borderWidth: 1,
    borderColor: colors.inkMuted,
    borderRadius: radius.sm,
  },
  checkboxChecked: { backgroundColor: colors.accent, borderColor: colors.accent },
  checkMark: { color: colors.surface, fontSize: 12, fontWeight: "700", lineHeight: 14 },
  table: {
    marginBottom: spacing.md,
    borderWidth: 1,
    borderColor: colors.lineSoft,
    borderRadius: radius.md,
    overflow: "hidden",
  },
  tableRow: { flexDirection: "row" },
  tableHead: { backgroundColor: colors.surfaceSubtle },
  tableBodyRow: { borderTopWidth: 1, borderTopColor: colors.lineSoft },
  tableRowZebra: { backgroundColor: colors.surfaceSubtle },
  th: {
    flexGrow: 1,
    flexBasis: 0,
    paddingHorizontal: spacing.md,
    paddingVertical: spacing.sm,
    color: colors.inkStrong,
    fontWeight: "700",
    fontSize: 14,
    lineHeight: 20,
  },
  td: {
    flexGrow: 1,
    flexBasis: 0,
    paddingHorizontal: spacing.md,
    paddingVertical: spacing.sm,
    color: colors.inkSoft,
    fontSize: 14,
    lineHeight: 20,
  },
  cellBorderLeft: { borderLeftWidth: 1, borderLeftColor: colors.lineSoft },
});
