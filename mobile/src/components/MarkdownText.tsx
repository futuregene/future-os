import type { InlineNode, ListItemNode, MarkdownNode } from "@future-os/markdown";
import type { ReactNode } from "react";
import { basename, localFilePath, parseFutureMarkdown } from "@future-os/markdown";
import { Linking, StyleSheet, Text, View } from "react-native";
import { colors, radius, spacing } from "../theme/tokens";

interface MarkdownTextProps {
  text: string;
  /** Route local-file markdown links/images to the caller's preview flow. */
  onOpenFile?(path: string): void;
}

type OpenFile = ((path: string) => void) | undefined;

function openTarget(target: string, onOpenFile: OpenFile) {
  const path = localFilePath(target);
  if (path && onOpenFile) {
    onOpenFile(path);
    return;
  }
  void Linking.openURL(target);
}

function renderInline(nodes: InlineNode[], onOpenFile: OpenFile, parentKey: string): ReactNode[] {
  return nodes.map((node, index) => {
    const key = `${parentKey}:in${index}`;
    switch (node.type) {
      case "strong":
        return (
          <Text key={key} style={styles.bold}>
            {renderInline(node.children, onOpenFile, key)}
          </Text>
        );
      case "italic":
        return (
          <Text key={key} style={styles.italic}>
            {renderInline(node.children, onOpenFile, key)}
          </Text>
        );
      case "delete":
        return (
          <Text key={key} style={styles.strike}>
            {renderInline(node.children, onOpenFile, key)}
          </Text>
        );
      case "code":
        return (
          <Text key={key} style={styles.inlineCode}>
            {node.code}
          </Text>
        );
      case "break":
        return "\n";
      case "link":
        return (
          <Text key={key} onPress={() => openTarget(node.href, onOpenFile)} style={styles.link}>
            {renderInline(node.children, onOpenFile, key)}
          </Text>
        );
      case "image": {
        const path = localFilePath(node.src);
        const label = node.alt || (path ? basename(path) : node.src);
        return (
          <Text key={key} onPress={() => openTarget(node.src, onOpenFile)} style={styles.link}>
            {label}
          </Text>
        );
      }
      case "futureReference": {
        const { reference } = node;
        const label = reference.label || basename(reference.targetId);
        if (reference.targetType !== "file") return label;
        return (
          <Text
            key={key}
            onPress={() => openTarget(reference.targetId, onOpenFile)}
            style={styles.link}
          >
            {label}
          </Text>
        );
      }
      default:
        return node.text;
    }
  });
}

function renderListItem(
  item: ListItemNode,
  itemIndex: number,
  ordered: boolean,
  onOpenFile: OpenFile,
  parentKey: string,
) {
  const key = `${parentKey}:item${itemIndex}`;
  return (
    <View key={key} style={styles.listRow}>
      {item.checked === undefined ? (
        <Text style={styles.listBullet}>{ordered ? `${itemIndex + 1}.` : "•"}</Text>
      ) : (
        <View style={[styles.checkbox, item.checked ? styles.checkboxChecked : null]}>
          {item.checked ? <Text style={styles.checkMark}>✓</Text> : null}
        </View>
      )}
      <View style={styles.listItemBody}>
        {item.children.length > 0 ? (
          <Text selectable style={styles.listItemText}>
            {renderInline(item.children, onOpenFile, key)}
          </Text>
        ) : null}
        {item.blocks?.length ? (
          <View style={styles.nestedBlocks}>
            {renderBlocks(item.blocks, onOpenFile, `${key}:blocks`)}
          </View>
        ) : null}
      </View>
    </View>
  );
}

function renderBlock(
  node: MarkdownNode,
  onOpenFile: OpenFile,
  key: string,
  isLast: boolean,
): ReactNode {
  switch (node.type) {
    case "heading":
      return (
        <Text
          key={key}
          selectable
          style={[
            styles.heading,
            node.level <= 2 ? styles.headingLarge : null,
            isLast ? styles.noBottom : null,
          ]}
        >
          {renderInline(node.children, onOpenFile, key)}
        </Text>
      );
    case "code":
      return (
        <Text key={key} selectable style={[styles.code, isLast ? styles.noBottom : null]}>
          {node.code}
        </Text>
      );
    case "blockquote":
      return (
        <View key={key} style={[styles.quote, isLast ? styles.noBottom : null]}>
          {renderBlocks(node.children, onOpenFile, `${key}:quote`)}
        </View>
      );
    case "list":
      return (
        <View key={key} style={[styles.list, isLast ? styles.noBottom : null]}>
          {node.items.map((item, index) =>
            renderListItem(item, index, node.ordered, onOpenFile, key),
          )}
        </View>
      );
    case "table":
      return (
        <View key={key} style={[styles.table, isLast ? styles.noBottom : null]}>
          <View style={[styles.tableRow, styles.tableHead]}>
            {node.headers.map((header, columnIndex) => (
              <Text
                key={`${key}:h${columnIndex}`}
                selectable
                style={[
                  styles.th,
                  columnIndex > 0 ? styles.cellBorderLeft : null,
                  { textAlign: node.alignments[columnIndex] ?? "left" },
                ]}
              >
                {renderInline(header, onOpenFile, `${key}:h${columnIndex}`)}
              </Text>
            ))}
          </View>
          {node.rows.map((row, rowIndex) => (
            <View
              key={`${key}:r${rowIndex}`}
              style={[
                styles.tableRow,
                styles.tableBodyRow,
                rowIndex % 2 === 1 ? styles.tableRowZebra : null,
              ]}
            >
              {row.map((cell, columnIndex) => (
                <Text
                  key={`${key}:r${rowIndex}:c${columnIndex}`}
                  selectable
                  style={[
                    styles.td,
                    columnIndex > 0 ? styles.cellBorderLeft : null,
                    { textAlign: node.alignments[columnIndex] ?? "left" },
                  ]}
                >
                  {renderInline(cell, onOpenFile, `${key}:r${rowIndex}:c${columnIndex}`)}
                </Text>
              ))}
            </View>
          ))}
        </View>
      );
    case "thematicBreak":
      return <View key={key} style={[styles.rule, isLast ? styles.noBottom : null]} />;
    case "futureEmbed": {
      const label = node.reference.label || basename(node.reference.targetId);
      return (
        <Text key={key} selectable style={[styles.paragraph, isLast ? styles.noBottom : null]}>
          <Text onPress={() => openTarget(node.reference.targetId, onOpenFile)} style={styles.link}>
            {label}
          </Text>
        </Text>
      );
    }
    default:
      return (
        <Text key={key} selectable style={[styles.paragraph, isLast ? styles.noBottom : null]}>
          {renderInline(node.children, onOpenFile, key)}
        </Text>
      );
  }
}

function renderBlocks(nodes: MarkdownNode[], onOpenFile: OpenFile, parentKey: string): ReactNode[] {
  return nodes.map((node, index) =>
    renderBlock(node, onOpenFile, `${parentKey}:b${index}`, index === nodes.length - 1),
  );
}

export function MarkdownText({ text, onOpenFile }: MarkdownTextProps) {
  const document = parseFutureMarkdown(text);
  return <View>{renderBlocks(document.nodes, onOpenFile, "markdown")}</View>;
}

const styles = StyleSheet.create({
  // The last block of a message drops its bottom margin — the surrounding
  // bubble/segment layout owns outer spacing.
  noBottom: { marginBottom: 0 },
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
    color: colors.ink,
    backgroundColor: colors.surfaceSubtle,
    fontFamily: "monospace",
    fontSize: 13,
    paddingHorizontal: 4,
    paddingVertical: 1,
    borderRadius: radius.sm,
    overflow: "hidden",
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
  listItemBody: { flex: 1 },
  listItemText: { color: colors.ink, fontSize: 17, lineHeight: 26 },
  nestedBlocks: { marginTop: spacing.xs },
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
