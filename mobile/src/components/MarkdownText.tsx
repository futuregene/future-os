import type { InlineNode, ListItemNode, MarkdownNode, TableNode } from "@future-os/markdown";
import type { ReactNode } from "react";
import {
  basename,
  classifyMarkdownTarget,
  localFilePath,
  createStreamingMarkdownParser,
  joinSoftBreaks,
  parseFutureMarkdown,
  remoteMarkdownImageUrl,
} from "@future-os/markdown";
import { memo, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import * as Clipboard from "expo-clipboard";
import { CodeTokens } from "./CodeTokens";
import { codePreviewRows, codeRowText } from "./codePreviewRows";
import { codeTokenRows, highlightCode } from "./codeHighlight";
import { markdownTableWidths } from "./markdownTableWidths";
import type { StyleProp, TextStyle } from "react-native";
import { Animated, FlatList, Linking, Platform, Pressable, ScrollView, StyleSheet, Text, useWindowDimensions, View } from "react-native";
import { AppAlert as Alert } from "./appAlerts";
import { useStreamingText } from "./useStreamingText";
import { chatTypography, colors, radius, spacing } from "../theme/tokens";
import { MarkdownImage, MarkdownImageBasePathContext, resolveMarkdownPath } from "./MarkdownImage";
import { MathFormula } from "./MathFormula";

interface MarkdownTextProps {
  /** Message links resolve against the workspace; a previewed document's links
   * resolve against that document, and a local file there is openable. */
  mode?: "message" | "file-preview";
  text: string;
  /** Original desktop document path, not its downloaded phone cache URI. */
  imageBasePath?: string;
  streaming?: boolean;
  /** Route local-file markdown links/images to the caller's preview flow. */
  onOpenFile?(path: string): void;
}

type OpenTarget = (target: string) => void;

/** Rows a table paints inline in the message. Past this it gets a bounded
 * viewport, which shows about eight rows at a time — so the cutoff must stay
 * above the tables a reply usually carries, or they silently lose their tail. */
const TABLE_INLINE_ROW_LIMIT = 20;

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
        return <MathFormula key={key} code={node.code} inline />;
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
        const label = node.alt || (path ? basename(path) : node.src);
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
            {node.children ? renderInline(node.children, openTarget, key) : label}
          </Text>
        );
      }
      default:
        return node.text;
    }
  });
}

type InlineRun = { nodes: InlineNode[] } | { image: Extract<InlineNode, { type: "image" }>; href?: string };

/** Native images cannot reliably participate in selectable Text layout. Split
 * them into responsive blocks, retaining formatting/link wrappers around the
 * surrounding text (including images nested in emphasis or external links).
 */
function inlineRuns(nodes: InlineNode[]): InlineRun[] {
  const runs: InlineRun[] = [];
  for (const node of nodes) {
    let parts: InlineRun[];
    if (node.type === "image" && (remoteMarkdownImageUrl(node.src) || localFilePath(node.src))) {
      parts = [{ image: node }];
    } else if ("children" in node && node.children) {
      parts = inlineRuns(node.children).map(run => "nodes" in run
        ? { nodes: [{ ...node, children: run.nodes }] }
        : { ...run, href: node.type === "link" ? node.href : node.type === "futureReference" ? node.reference.targetId : run.href });
    } else {
      parts = [{ nodes: [node] }];
    }
    for (const part of parts) {
      const last = runs[runs.length - 1];
      if (last && "nodes" in last && "nodes" in part) last.nodes.push(...part.nodes);
      else runs.push(part);
    }
  }
  return runs;
}

function InlineContent({ nodes, openTarget, textStyle, heading = false }: {
  nodes: InlineNode[];
  openTarget: OpenTarget;
  textStyle: StyleProp<TextStyle>;
  heading?: boolean;
}) {
  return inlineRuns(nodes).map((run, index) => {
    if ("nodes" in run) return (
      <Text key={index} selectable accessibilityRole={heading ? "header" : undefined} style={textStyle}>
        {renderInline(run.nodes, openTarget, `run${index}`)}
      </Text>
    );
    return <MarkdownImage key={index} alt={run.image.alt} src={run.image.src} href={run.href} openTarget={openTarget} />;
  });
}

/** Collapsed height of a long block, in wrapped lines. The source chunks bound
 * the mounted characters; this bounds the painted height, which one long CJK
 * paragraph can otherwise blow past. 16 keeps the previous fixed viewport's
 * worth of text (its 360dp held ~17 unwrapped lines) without clipping a line. */
const collapsedCodeLines = 16;

function CodeSource({ code, language }: { code: string; language?: string }) {
  const { t } = useTranslation();
  const [expanded, setExpanded] = useState(false);
  const large = code.length > 10000 || code.split("\n", 18).length > 16;
  const rows = useMemo(() => large ? codePreviewRows(code) : [], [code, large]);
  const tokens = useMemo(() => highlightCode(code, language), [code, language]);
  const rowTokens = useMemo(() => codeTokenRows(tokens, rows), [tokens, rows]);
  // Wrapping (never a horizontal scroll view) keeps a phone-width block
  // readable; an explicit toggle, not a nested vertical viewport, is what makes
  // the tail of a long block reachable — an inner scroll region loses the
  // gesture to the surrounding message list.
  const collapsed = large && !expanded;
  return (
    <View style={styles.codeContainer}>
      {large ? (
        <View style={styles.codeToolbar}>
          <Text style={styles.codeLanguage}>{language ?? ""}</Text>
          <Pressable accessibilityRole="button" accessibilityLabel={t("chat.copy")}
            onPress={() => { void Clipboard.setStringAsync(code).catch(error =>
              Alert.alert(t("common.error"), error instanceof Error ? error.message : String(error))); }} style={styles.codeCopy}>
            <Text>{t("chat.copy")}</Text>
          </Pressable>
        </View>
      ) : language ? <Text style={styles.codeLanguage}>{language}</Text> : null}
      {large ? rows.slice(0, collapsed ? 1 : rows.length).map((row, index) => (
        <Text key={index} selectable style={styles.code} numberOfLines={collapsed ? collapsedCodeLines : undefined}
          ellipsizeMode={collapsed ? "tail" : undefined}>
          {row.continuation ? "↪ " : ""}
          <CodeTokens fallback={codeRowText(row.text)} tokens={rowTokens[index] ?? null} />
        </Text>
      )) : (
        <Text selectable style={styles.code}><CodeTokens fallback={code} tokens={tokens} /></Text>
      )}
      {large ? (
        <Pressable accessibilityRole="button"
          accessibilityLabel={t(collapsed ? "chat.expandCode" : "chat.collapseCode")}
          onPress={() => setExpanded(value => !value)} style={styles.codeToggle}>
          <Text>{t(collapsed ? "chat.expandCode" : "chat.collapseCode")}</Text>
        </Pressable>
      ) : null}
    </View>
  );
}

function renderListItem(
  item: ListItemNode,
  itemIndex: number,
  ordered: boolean,
  start: number,
  openTarget: OpenTarget,
  parentKey: string,
) {
  const key = `${parentKey}:item${itemIndex}`;
  return (
    <View key={key} style={styles.listRow}>
      {item.checked === undefined ? (
        <Text style={styles.listBullet}>{ordered ? `${start + itemIndex}.` : "•"}</Text>
      ) : (
        <View accessibilityRole="checkbox" accessibilityState={{ checked: item.checked, disabled: true }} style={[styles.checkbox, item.checked ? styles.checkboxChecked : null]}>
          {item.checked ? <Text style={styles.checkMark}>✓</Text> : null}
        </View>
      )}
      <View style={styles.listItemBody}>
        {item.children.length > 0 ? (
          <InlineContent nodes={item.children} openTarget={openTarget} textStyle={styles.listItemText} />
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
        <View key={key} style={isLast ? undefined : styles.blockSpacing}>
          <InlineContent nodes={node.children} openTarget={openTarget} heading textStyle={[styles.heading, headingSizes[node.level - 1]!]} />
        </View>
      );
    case "code":
      return (
        <View key={key} style={isLast ? undefined : styles.blockSpacing}>
          <CodeSource code={node.code} language={node.language} />
        </View>
      );
    case "mathBlock":
      return (
        <View key={key} style={isLast ? undefined : styles.blockSpacing}>
          <MathFormula code={node.code} />
        </View>
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
            renderListItem(item, index, node.ordered, node.start ?? 1, openTarget, key),
          )}
        </View>
      );
    case "table":
      return (
        <View key={key} style={isLast ? undefined : styles.blockSpacing}>
          <MarkdownTable node={node} openTarget={openTarget} />
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
        <View key={key} style={isLast ? undefined : styles.blockSpacing}>
          <InlineContent nodes={node.children} openTarget={openTarget} textStyle={styles.bodyText} />
        </View>
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

function MarkdownTable({ node, openTarget }: { node: TableNode; openTarget: OpenTarget }) {
  const { t } = useTranslation();
  const [width, setWidth] = useState(0);
  const { fontScale } = useWindowDimensions();
  const cellWidths = useMemo(() => markdownTableWidths(node, width, fontScale), [node, width, fontScale]);
  const tableWidth = cellWidths.reduce((sum, cellWidth) => sum + cellWidth, 0);
  const renderRow = useCallback(({ item, index }: { item: InlineNode[][]; index: number }) => (
    <MarkdownTableRow cells={item} alignments={node.alignments} cellWidths={cellWidths} openTarget={openTarget} striped={index % 2 === 1} />
  ), [cellWidths, node.alignments, openTarget]);
  const bounded = node.rows.length > TABLE_INLINE_ROW_LIMIT;
  return (
    <View style={styles.constrained} onLayout={event => setWidth(event.nativeEvent.layout.width)}>
      <ScrollView horizontal nestedScrollEnabled>
        <View style={styles.table}>
          <MarkdownTableRow cells={node.headers} alignments={node.alignments} cellWidths={cellWidths} openTarget={openTarget} header />
          {bounded ? (
            <FlatList data={node.rows} renderItem={renderRow} nestedScrollEnabled
              initialNumToRender={12} maxToRenderPerBatch={12} windowSize={5}
              style={{ height: 360, width: tableWidth }}
              keyExtractor={(_row, index) => String(index)} />
          ) : node.rows.map((row, rowIndex) => (
            <MarkdownTableRow key={rowIndex} cells={row} alignments={node.alignments} cellWidths={cellWidths} openTarget={openTarget} striped={rowIndex % 2 === 1} />
          ))}
        </View>
      </ScrollView>
      {/* A bounded viewport clips rows: say so, or the table looks complete. */}
      {bounded ? (
        <Text style={styles.tableRowsHint}>{t("chat.tableRowsScrolled", { count: node.rows.length })}</Text>
      ) : null}
    </View>
  );
}

const MarkdownTableRow = memo(function MarkdownTableRow({ cells, alignments, cellWidths, openTarget, header = false, striped = false }: {
  cells: InlineNode[][];
  alignments: TableNode["alignments"];
  cellWidths: number[];
  openTarget: OpenTarget;
  header?: boolean;
  striped?: boolean;
}) {
  return (
    <View style={[styles.tableRow, header ? styles.tableHead : styles.tableBodyRow, striped && styles.tableRowZebra]}>
      {cells.map((cell, index) => (
        <View key={index} style={[
          styles.tableCell,
          index > 0 ? styles.cellBorderLeft : null,
          { width: cellWidths[index] },
        ]}>
          <InlineContent nodes={cell} openTarget={openTarget} textStyle={[
            header ? styles.th : styles.td,
            { textAlign: alignments[index] ?? "left" },
          ]} />
        </View>
      ))}
    </View>
  );
});

const MarkdownBlock = memo(function MarkdownBlock({ node, openTarget, isLast, animate }: {
  node: MarkdownNode;
  openTarget: OpenTarget;
  isLast: boolean;
  animate: boolean;
}) {
  const [opacity] = useState(() => new Animated.Value(animate ? 0 : 1));
  const appeared = useRef(false);
  useEffect(() => {
    if (!animate || appeared.current) {
      opacity.setValue(1);
      return;
    }
    appeared.current = true;
    opacity.setValue(0);
    const animation = Animated.timing(opacity, {
      toValue: 1, duration: 180, useNativeDriver: true, isInteraction: false,
    });
    animation.start();
    return () => animation.stop();
  }, [animate, opacity]);
  // Only new blocks fade once. Updating a token never fades the whole message.
  return <Animated.View style={{ opacity }}>{renderBlock(node, openTarget, "block", isLast)}</Animated.View>;
});

export function MarkdownText({ text, onOpenFile, imageBasePath, mode = "message", streaming = false }: MarkdownTextProps) {
  const { t } = useTranslation();
  const [project] = useState(createStreamingMarkdownParser);
  const reveal = useStreamingText(text, mode === "message" && streaming);
  const displayedText = mode === "message" ? reveal.text : text;
  const projectingStream = streaming || displayedText !== text;
  const document = useMemo(() => {
    if (mode !== "file-preview") return project(displayedText, projectingStream);
    // A previewed document reflows to the reader's width: its source soft breaks
    // join into spaces instead of showing the wrap column of the file. Chat
    // keeps the author's newlines.
    // Large file ASTs should die with the preview, not occupy the shared
    // 512-entry message cache after the modal closes.
    return joinSoftBreaks(parseFutureMarkdown(displayedText, undefined, displayedText.length <= 128 * 1024));
  }, [mode, project, displayedText, projectingStream]);
  const [initialBlockCount] = useState(document.nodes.length);
  const openTarget = useCallback<OpenTarget>(rawTarget => {
    const target = classifyMarkdownTarget(rawTarget);
    if (target.kind === "local-file") {
      // In a previewed document a link is relative to that document, not to the
      // workspace the desktop resolves chat links against. A document opened
      // from this phone has no desktop-side directory to resolve against.
      const path = mode === "file-preview"
        ? resolveMarkdownPath(target.path, imageBasePath)
        : target.path;
      if (!path) {
        Alert.alert(t("attachment.title"), t("attachment.localLinkUnresolvable"));
        return;
      }
      onOpenFile?.(path);
      return;
    }
    if (target.kind !== "external-url") return;
    void Linking.openURL(target.url).catch(() => {
      Alert.alert(t("attachment.title"), t("attachment.linkOpenFailed"));
    });
  }, [imageBasePath, mode, onOpenFile, t]);
  const renderPreviewBlock = useCallback(({ item, index }: { item: MarkdownNode; index: number }) => (
    <MarkdownBlock node={item} openTarget={openTarget} isLast={index === document.nodes.length - 1} animate={false} />
  ), [document.nodes.length, openTarget]);
  if (mode === "file-preview") return <MarkdownImageBasePathContext value={imageBasePath}>
    <FlatList
      data={document.nodes}
      renderItem={renderPreviewBlock}
      keyExtractor={(_item, index) => String(index)}
      initialNumToRender={8}
      maxToRenderPerBatch={8}
      windowSize={5}
      style={styles.previewList}
      contentContainerStyle={styles.previewContent}
    />
  </MarkdownImageBasePathContext>;
  return <MarkdownImageBasePathContext value={imageBasePath}><View style={styles.constrained}>{document.nodes.map((node, index) => (
    <MarkdownBlock key={index} node={node} openTarget={openTarget} isLast={index === document.nodes.length - 1}
      animate={mode === "message" && projectingStream && !reveal.reduceMotion && index >= initialBlockCount} />
  ))}</View></MarkdownImageBasePathContext>;
}

const monospace = Platform.select({ ios: "Menlo", default: "monospace" });
const headingSizes: TextStyle[] = [
  { fontSize: 24, lineHeight: 32 },
  { fontSize: 21, lineHeight: 29 },
  { fontSize: 19, lineHeight: 27 },
  { fontSize: 17, lineHeight: 25 },
  { fontSize: 16, lineHeight: 24 },
  { fontSize: 15, lineHeight: 22 },
];

const styles = StyleSheet.create({
  // The last block of a message drops its bottom margin — the surrounding
  // bubble/segment layout owns outer spacing.
  noBottom: { marginBottom: 0 },
  constrained: { minWidth: 0, maxWidth: "100%", alignSelf: "stretch" },
  previewList: { flex: 1, minWidth: 0, width: "100%" },
  // The preview's gutter belongs to the scrolling content: padding on a static
  // parent instead leaves a blank strip under the header once text scrolls.
  previewContent: { padding: spacing.lg, minWidth: 0, maxWidth: "100%", alignSelf: "stretch" },
  blockSpacing: { marginBottom: spacing.sm },
  bodyText: { color: colors.ink, ...chatTypography },
  paragraph: { color: colors.ink, ...chatTypography, marginBottom: spacing.sm },
  heading: {
    color: colors.inkStrong,
    fontWeight: "700",
  },
  bold: { fontWeight: "700" },
  italic: { fontStyle: "italic" },
  strike: { textDecorationLine: "line-through" },
  inlineCode: {
    color: colors.ink,
    backgroundColor: colors.surfaceSubtle,
    fontFamily: monospace,
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
  rule: { height: 1, marginVertical: spacing.md, backgroundColor: colors.line },
  codeContainer: {
    maxWidth: "100%",
    borderRadius: radius.md,
    overflow: "hidden",
    backgroundColor: colors.surfaceSubtle,
  },
  codeToolbar: { flexDirection: "row", alignItems: "center", justifyContent: "space-between" },
  codeCopy: { padding: spacing.md, minHeight: 44, justifyContent: "center" },
  codeToggle: { padding: spacing.sm, minHeight: 44, alignItems: "center", justifyContent: "center", borderTopWidth: 1, borderTopColor: colors.lineSoft },
  codeLanguage: { color: colors.inkMuted, fontSize: 12, paddingHorizontal: spacing.md, paddingTop: spacing.sm },
  code: {
    padding: spacing.md,
    color: colors.ink,
    fontFamily: monospace,
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
    minWidth: 22,
    flexShrink: 0,
    textAlign: "right",
    marginRight: spacing.xs,
    color: colors.ink,
    ...chatTypography,
  },
  listItemBody: { flex: 1, minWidth: 0 },
  listItemText: { color: colors.ink, ...chatTypography },
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
    borderWidth: 1,
    borderColor: colors.lineSoft,
    borderRadius: radius.md,
    overflow: "hidden",
  },
  tableRow: { flexDirection: "row" },
  tableHead: { backgroundColor: colors.surfaceSubtle },
  tableBodyRow: { borderTopWidth: 1, borderTopColor: colors.lineSoft },
  tableRowZebra: { backgroundColor: colors.surfaceSubtle },
  tableRowsHint: { paddingHorizontal: spacing.sm, paddingTop: spacing.xs, color: colors.inkMuted, fontSize: 12 },
  tableCell: { paddingHorizontal: spacing.sm, paddingVertical: spacing.sm },
  th: {
    color: colors.inkStrong,
    fontWeight: "700",
    fontSize: 14,
    lineHeight: 20,
  },
  td: {
    color: colors.inkSoft,
    fontSize: 14,
    lineHeight: 20,
  },
  cellBorderLeft: { borderLeftWidth: 1, borderLeftColor: colors.lineSoft },
});
