import type { InlineNode, ListItemNode, MarkdownNode } from "@future-os/markdown";
import type { ReactNode } from "react";
import {
  basename,
  classifyMarkdownTarget,
  localFilePath,
  parseFutureMarkdown,
  remoteMarkdownImageUrl,
} from "@future-os/markdown";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Alert, Image, Linking, StyleSheet, Text, View } from "react-native";
import { colors, radius, spacing } from "../theme/tokens";

interface MarkdownTextProps {
  /** Message links can fetch local files; file previews never nest previews. */
  mode?: "message" | "file-preview";
  text: string;
  /** Route local-file markdown links/images to the caller's preview flow. */
  onOpenFile?(path: string): void;
}

type OpenTarget = (target: string) => void;

function renderInline(nodes: InlineNode[], openTarget: OpenTarget, parentKey: string): ReactNode[] {
  return nodes.map((node, index) => {
    const key = `${parentKey}:in${index}`;
    switch (node.type) {
      case "strong":
        return (
          <Text key={key} style={styles.bold}>
            {renderInline(node.children, openTarget, key)}
          </Text>
        );
      case "italic":
        return (
          <Text key={key} style={styles.italic}>
            {renderInline(node.children, openTarget, key)}
          </Text>
        );
      case "delete":
        return (
          <Text key={key} style={styles.strike}>
            {renderInline(node.children, openTarget, key)}
          </Text>
        );
      case "code":
        return (
          <Text key={key} style={styles.inlineCode}>
            {node.code}
          </Text>
        );
      case "mathInline":
        // React Native has no KaTeX DOM renderer; fall back to the raw TeX
        // source in monospace so formulas stay legible.
        return (
          <Text key={key} style={styles.inlineCode}>
            {node.code}
          </Text>
        );
      case "break":
        return "\n";
      case "link": {
        const target = classifyMarkdownTarget(node.href);
        if (target.kind === "blocked" || target.kind === "document-anchor")
          return <Text key={key}>{renderInline(node.children, openTarget, key)}</Text>;
        return (
          <Text key={key} onPress={() => openTarget(node.href)} style={styles.link}>
            {renderInline(node.children, openTarget, key)}
          </Text>
        );
      }
      case "image": {
        const path = localFilePath(node.src);
        const remoteUrl = remoteMarkdownImageUrl(node.src);
        const label = node.alt || (path ? basename(path) : node.src);
        if (remoteUrl) {
          return <RemoteMarkdownImage alt={node.alt} key={key} url={remoteUrl} />;
        }
        if (!path) return label;
        return (
          <Text key={key} onPress={() => openTarget(node.src)} style={styles.fileChip}>
            {label}
          </Text>
        );
      }
      case "futureReference": {
        const { reference } = node;
        const label = reference.label || basename(reference.targetId);
        if (reference.targetType !== "file") return label;
        return (
          <Text key={key} onPress={() => openTarget(reference.targetId)} style={styles.link}>
            {label}
          </Text>
        );
      }
      default:
        return node.text;
    }
  });
}

function RemoteMarkdownImage({ alt, url }: { alt: string; url: string }) {
  const [failedUrl, setFailedUrl] = useState<string | null>(null);
  const failed = failedUrl === url;
  if (failed) return <Text style={styles.imageFallback}>{alt || url}</Text>;
  return (
    <Image
      accessibilityLabel={alt}
      onError={() => setFailedUrl(url)}
      resizeMode="contain"
      source={{ uri: url }}
      style={styles.remoteImage}
    />
  );
}

function renderListItem(
  item: ListItemNode,
  itemIndex: number,
  ordered: boolean,
  openTarget: OpenTarget,
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
            {renderInline(item.children, openTarget, key)}
          </Text>
        ) : null}
        {item.blocks?.length ? (
          <View style={styles.nestedBlocks}>
            {renderBlocks(item.blocks, openTarget, `${key}:blocks`)}
          </View>
        ) : null}
      </View>
    </View>
  );
}

function renderBlock(
  node: MarkdownNode,
  openTarget: OpenTarget,
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
          {renderInline(node.children, openTarget, key)}
        </Text>
      );
    case "code":
      return (
        <Text key={key} selectable style={[styles.code, isLast ? styles.noBottom : null]}>
          {node.code}
        </Text>
      );
    case "mathBlock":
      // React Native has no KaTeX DOM renderer; fall back to the raw TeX
      // source in the block-code style so formulas stay legible.
      return (
        <Text key={key} selectable style={[styles.code, isLast ? styles.noBottom : null]}>
          {node.code}
        </Text>
      );
    case "blockquote":
      return (
        <View key={key} style={[styles.quote, isLast ? styles.noBottom : null]}>
          {renderBlocks(node.children, openTarget, `${key}:quote`)}
        </View>
      );
    case "list":
      return (
        <View key={key} style={[styles.list, isLast ? styles.noBottom : null]}>
          {node.items.map((item, index) =>
            renderListItem(item, index, node.ordered, openTarget, key),
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
                {renderInline(header, openTarget, `${key}:h${columnIndex}`)}
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
                  {renderInline(cell, openTarget, `${key}:r${rowIndex}:c${columnIndex}`)}
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
          <Text onPress={() => openTarget(node.reference.targetId)} style={styles.fileChip}>
            {label}
          </Text>
        </Text>
      );
    }
    default:
      return (
        <Text key={key} selectable style={[styles.paragraph, isLast ? styles.noBottom : null]}>
          {renderInline(node.children, openTarget, key)}
        </Text>
      );
  }
}

function renderBlocks(
  nodes: MarkdownNode[],
  openTarget: OpenTarget,
  parentKey: string,
): ReactNode[] {
  return nodes.map((node, index) =>
    renderBlock(node, openTarget, `${parentKey}:b${index}`, index === nodes.length - 1),
  );
}

export function MarkdownText({ text, onOpenFile, mode = "message" }: MarkdownTextProps) {
  const { t } = useTranslation();
  const document = parseFutureMarkdown(text);
  const openTarget: OpenTarget = rawTarget => {
    const target = classifyMarkdownTarget(rawTarget);
    if (target.kind === "local-file") {
      if (mode === "file-preview") {
        Alert.alert(t("attachment.title"), t("attachment.localLinkDesktopOnly"));
      } else {
        onOpenFile?.(target.path);
      }
      return;
    }
    if (target.kind !== "external-url") return;
    void Linking.openURL(target.url).catch(() => {
      Alert.alert(t("attachment.title"), t("attachment.linkOpenFailed"));
    });
  };
  return <View>{renderBlocks(document.nodes, openTarget, "markdown")}</View>;
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
  fileChip: {
    color: colors.accent,
    backgroundColor: colors.surfaceSubtle,
    borderRadius: radius.sm,
    paddingHorizontal: 5,
    paddingVertical: 2,
    overflow: "hidden",
  },
  remoteImage: {
    width: 240,
    height: 160,
    marginVertical: spacing.sm,
    borderRadius: radius.md,
    backgroundColor: colors.surfaceSubtle,
  },
  imageFallback: { color: colors.inkMuted },
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
