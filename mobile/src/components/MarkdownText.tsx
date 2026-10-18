import type { InlineNode, ListItemNode, MarkdownNode, TableNode } from "@future-os/markdown";
import type { ReactNode } from "react";
import {
  basename,
  classifyMarkdownTarget,
  localFilePath,
  createStreamingMarkdownParser,
  joinSoftBreaks,
  parseFutureMarkdown,
  parseSessionReferenceHref,
  remoteMarkdownImageUrl,
} from "@future-os/markdown";
import { createContext, memo, useCallback, useContext, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import * as Clipboard from "expo-clipboard";
import { CodeTokens } from "./CodeTokens";
import { codePreviewRows, codeRowText } from "./codePreviewRows";
import { codeTokenRows, highlightCode } from "./codeHighlight";
import { markdownTableWidths } from "./markdownTableWidths";
import { MarkdownToneContext, type MarkdownTone } from "./markdownTone";
import type { StyleProp, TextStyle } from "react-native";
import { Animated, FlatList, Linking, Platform, Pressable, ScrollView, StyleSheet, Text, useWindowDimensions, View } from "react-native";
import { AppAlert as Alert } from "./appAlerts";
import { useStreamingText } from "./useStreamingText";
import { radius, spacing } from "../theme/tokens";
import type { Theme } from "../theme/theme";
import { useTheme } from "../theme/ThemeContext";
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
  /** Opens the conversation a `#` reference points at. Unset leaves the
   * reference a plain chip (what a previewed document does today). */
  onOpenSession?(sessionId: string): void;
  /** The surface this body is drawn on (see `MarkdownTone`). */
  tone?: MarkdownTone;
}

type OpenTarget = (target: string) => void;

/** Where a `#` reference opens. Its own context: the chip is built deep inside
 * the plain block renderers, which cannot take a prop for it. */
const MarkdownSessionContext = createContext<((sessionId: string) => void) | undefined>(undefined);

/** Rows a table paints before it asks the reader. Past this the table paints
 * this many rows and offers the rest behind an explicit control — a nested
 * vertical viewport inside the list around it loses the pan gesture (see the
 * code block below) — so the cutoff must stay above the tables a reply usually
 * carries, or they silently lose their tail. */
const TABLE_INLINE_ROW_LIMIT = 20;

/**
 * A `#` reference to another conversation.
 *
 * The composer writes a picked conversation as `[Title](futureos://session/<id>)`
 * — a plain markdown link, so the id travels to the model as ordinary text — so
 * every body that renders the prompt back sees a link node rather than a special
 * one. The chip is what keeps the `#` marker the composer shows: without it the
 * reference reads as a bare title, indistinguishable from a file mention at a
 * glance. Tapping it moves to that conversation, when the screen can.
 */
const SessionChip = memo(function SessionChip({ href, children }: { href: string; children: ReactNode; }) {
  const styles = useStyles();
  const open = useContext(MarkdownSessionContext);
  const sessionId = parseSessionReferenceHref(href);
  return (
    <Text
      accessibilityRole="link"
      onPress={open && sessionId ? () => open(sessionId) : undefined}
      style={styles.sessionChip}
    >
      #{children}
    </Text>
  );
});

function renderInline(nodes: InlineNode[], openTarget: OpenTarget, parentKey: string, styles: MarkdownStyles): ReactNode[] {
  return nodes.map((node, index) => {
    const key = `${parentKey}:in${index}`;
    switch (node.type) {
      case "strong":
        return (
          <Text key={key} style={styles.bold}>
            {renderInline(node.children, openTarget, key, styles)}
          </Text>
        );
      case "italic":
        return (
          <Text key={key} style={styles.italic}>
            {renderInline(node.children, openTarget, key, styles)}
          </Text>
        );
      case "delete":
        return (
          <Text key={key} style={styles.strike}>
            {renderInline(node.children, openTarget, key, styles)}
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
        // A `#` conversation reference is a plain markdown link whose
        // destination is the session id (see `@future-os/markdown`), which is
        // why it has to be recognised before the general link handling: the
        // scheme is not one a phone can open, so it would fall through to the
        // blocked branch and lose both its marker and its tap.
        if (parseSessionReferenceHref(node.href))
          return <SessionChip href={node.href} key={key}>{renderInline(node.children, openTarget, key, styles)}</SessionChip>;
        const target = classifyMarkdownTarget(node.href);
        if (target.kind === "blocked" || target.kind === "document-anchor")
          return <Text key={key}>{renderInline(node.children, openTarget, key, styles)}</Text>;
        return (
          <Text key={key} onPress={() => openTarget(node.href)} style={styles.link}>
            {renderInline(node.children, openTarget, key, styles)}
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
            {node.children ? renderInline(node.children, openTarget, key, styles) : label}
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

function InlineContent({ nodes, openTarget, textStyle, heading = false, align }: {
  nodes: InlineNode[];
  openTarget: OpenTarget;
  textStyle: StyleProp<TextStyle>;
  heading?: boolean;
  align?: "center" | "left" | "right";
}) {
  const styles = useStyles();
  const alignStyle = align ? { textAlign: align } : undefined;
  return inlineRuns(nodes).map((run, index) => {
    if ("nodes" in run) return (
      <Text key={index} selectable accessibilityRole={heading ? "header" : undefined} style={[textStyle, alignStyle]}>
        {renderInline(run.nodes, openTarget, `run${index}`, styles)}
      </Text>
    );
    return <MarkdownImage key={index} alt={run.image.alt} src={run.image.src} width={run.image.width} href={run.href} openTarget={openTarget} />;
  });
}

/** Collapsed height of a long block, in wrapped lines. The source chunks bound
 * the mounted characters; this bounds the painted height, which one long CJK
 * paragraph can otherwise blow past. 16 keeps the previous fixed viewport's
 * worth of text (its 360dp held ~17 unwrapped lines) without clipping a line. */
const collapsedCodeLines = 16;

function CodeSource({ code, language }: { code: string; language?: string }) {
  const styles = useStyles();
  const { codeColors } = useTheme();
  const { t } = useTranslation();
  const tone = useContext(MarkdownToneContext);
  const [expanded, setExpanded] = useState(false);
  const large = code.length > 10000 || code.split("\n", 18).length > 16;
  const rows = useMemo(() => large ? codePreviewRows(code) : [], [code, large]);
  // A syntax palette is drawn for a light/dark *surface*, and the user's own
  // bubble is neither: on the accent fill the same tokens would drop contrast
  // haphazardly, so that tone paints the block in one readable colour instead
  // (which `CodeTokens` does for a null token list).
  const tokens = useMemo(
    () => tone === "onAccent" ? null : highlightCode(code, language, codeColors),
    [code, language, codeColors, tone],
  );
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
  styles: MarkdownStyles,
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
            {renderBlocks(item.blocks, openTarget, `${key}:blocks`, styles)}
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
  styles: MarkdownStyles,
): ReactNode {
  // Heading sizes scale with the reader's text size like the sheet's own styles,
  // so they are indexed out of it rather than held in a module constant.
  const headingSizes = [styles.h1, styles.h2, styles.h3, styles.h4, styles.h5, styles.h6];
  // A block's own alignment (`<p align>`, `<h3 align>` from raw HTML): text
  // centers via `textAlign`, and a narrower-than-container image centers via the
  // wrapper's `alignItems`. A centered block also keeps its usual bottom margin;
  // the last block drops it.
  const blockAlign = node.type === "paragraph" || node.type === "heading"
    ? node.align
    : undefined;
  const baseMargin = isLast ? undefined : styles.blockSpacing;
  const wrapperStyle = blockAlign === "center"
    ? (baseMargin ? [baseMargin, { alignItems: "center" as const }] : { alignItems: "center" as const })
    : baseMargin;
  switch (node.type) {
    case "heading":
      return (
        <View key={key} style={wrapperStyle}>
          <InlineContent nodes={node.children} openTarget={openTarget} heading align={blockAlign} textStyle={[styles.heading, headingSizes[node.level - 1]!]} />
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
          {renderBlocks(node.children, openTarget, `${key}:quote`, styles)}
        </View>
      );
    case "list":
      return (
        <View key={key} style={[styles.list, isLast ? styles.noBottom : null]}>
          {node.items.map((item, index) =>
            renderListItem(item, index, node.ordered, node.start ?? 1, openTarget, key, styles),
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
        <View key={key} style={wrapperStyle}>
          <InlineContent nodes={node.children} openTarget={openTarget} align={blockAlign} textStyle={styles.bodyText} />
        </View>
      );
  }
}

function renderBlocks(
  nodes: MarkdownNode[],
  openTarget: OpenTarget,
  parentKey: string,
  styles: MarkdownStyles,
): ReactNode[] {
  return nodes.map((node, index) =>
    renderBlock(node, openTarget, `${parentKey}:b${index}`, index === nodes.length - 1, styles),
  );
}

function MarkdownTable({ node, openTarget }: { node: TableNode; openTarget: OpenTarget }) {
  const styles = useStyles();
  const { t } = useTranslation();
  const [width, setWidth] = useState(0);
  const [expanded, setExpanded] = useState(false);
  const { fontScale } = useWindowDimensions();
  const cellWidths = useMemo(() => markdownTableWidths(node, width, fontScale), [node, width, fontScale]);
  const bounded = node.rows.length > TABLE_INLINE_ROW_LIMIT;
  const collapsed = bounded && !expanded;
  // Every painted row is a real row of the table, wrapped by the message list
  // like any other block. The 360dp inner viewport this replaces put most of a
  // long table behind a scroll region the surrounding list takes the drag for,
  // so its tail could not be reached at all. The horizontal scroller stays —
  // a sideways pan is not contended for, and a wide table still needs it.
  const rows = collapsed ? node.rows.slice(0, TABLE_INLINE_ROW_LIMIT) : node.rows;
  const label = t(collapsed ? "chat.tableRowsExpand" : "chat.tableRowsCollapse", { count: node.rows.length });
  return (
    <View style={styles.constrained} onLayout={event => setWidth(event.nativeEvent.layout.width)}>
      <ScrollView horizontal nestedScrollEnabled>
        <View style={styles.table}>
          <MarkdownTableRow cells={node.headers} alignments={node.alignments} cellWidths={cellWidths} openTarget={openTarget} header />
          {rows.map((row, rowIndex) => (
            <MarkdownTableRow key={rowIndex} cells={row} alignments={node.alignments} cellWidths={cellWidths} openTarget={openTarget} striped={rowIndex % 2 === 1} />
          ))}
        </View>
      </ScrollView>
      {/* Rows held back make the table look complete: say how many it holds and
          make the rest one tap away. The control sits outside the horizontal
          scroller, so it stays on screen when the table is wider than the phone. */}
      {bounded ? (
        <Pressable accessibilityLabel={label} accessibilityRole="button"
          onPress={() => setExpanded(value => !value)} style={styles.tableRowsToggle}>
          <Text style={styles.tableRowsHint}>{label}</Text>
        </Pressable>
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
  const styles = useStyles();
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
  const styles = useStyles();
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
  return <Animated.View style={{ opacity }}>{renderBlock(node, openTarget, "block", isLast, styles)}</Animated.View>;
});

export function MarkdownText({ text, onOpenFile, onOpenSession, imageBasePath, mode = "message", streaming = false, tone = "default" }: MarkdownTextProps) {
  const styles = useStyles();
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
  return <MarkdownImageBasePathContext value={imageBasePath}>
    <MarkdownToneContext value={tone}>
      <MarkdownSessionContext value={onOpenSession}>
        <View style={styles.constrained}>{document.nodes.map((node, index) => (
          <MarkdownBlock key={index} node={node} openTarget={openTarget} isLast={index === document.nodes.length - 1}
            animate={mode === "message" && projectingStream && !reveal.reduceMotion && index >= initialBlockCount} />
        ))}</View>
      </MarkdownSessionContext>
    </MarkdownToneContext>
  </MarkdownImageBasePathContext>;
}

const monospace = Platform.select({ ios: "Menlo", default: "monospace" });

/** The markdown sheet, named because the module-level block renderers take it as
 * an argument: they are plain functions, so they cannot call `useStyles`. */
type MarkdownStyles = ReturnType<typeof buildStyles>;

/** One sheet per theme **and tone**: the same body is drawn on the app's own
 * surface and, when it is the user's own bubble, on the accent fill. */
const sheets = new WeakMap<Theme, Partial<Record<MarkdownTone, MarkdownStyles>>>();

function useStyles(): MarkdownStyles {
  const theme = useTheme();
  const tone = useContext(MarkdownToneContext);
  let byTone = sheets.get(theme);
  if (!byTone)
    byTone = sheets.set(theme, {}).get(theme)!;
  const cached = byTone[tone];
  if (cached)
    return cached;
  const sheet = buildStyles(theme, tone);
  byTone[tone] = sheet;
  return sheet;
}

function buildStyles(theme: Theme, tone: MarkdownTone) {
  /**
   * On the accent fill the palette no longer applies.
   *
   * `ink` on the accent colour is unreadable and `surfaceSubtle` is not a recess
   * on it, so the on-accent tone paints every foreground in the bubble's own
   * contrast colour (`surface`, which is what the bubble's text already uses) and
   * every recess as a translucent wash of it: a code block, a chip and a table
   * head read as a raised pane of the bubble rather than as a hole in it.
   */
  const onAccent = tone === "onAccent";
  const text = onAccent ? theme.colors.surface : theme.colors.ink;
  const textStrong = onAccent ? theme.colors.surface : theme.colors.inkStrong;
  const textSoft = onAccent ? theme.colors.surface : theme.colors.inkSoft;
  const textMuted = onAccent ? theme.colors.surface : theme.colors.inkMuted;
  const accent = onAccent ? theme.colors.surface : theme.colors.accent;
  // The wash is a translation of the bubble's *own* contrast colour, so it works
  // in both schemes without asking which one is painted: the light scheme's
  // accent is dark with white text (a white wash, alpha 0x33 ≈ 20%), the dark
  // scheme's is light with dark text (a darkening wash of the same colour).
  // Appending an alpha byte is how `surface` — `#rrggbb` in both palettes —
  // becomes one.
  const recess = onAccent ? `${theme.colors.surface}33` : theme.colors.surfaceSubtle;
  const line = onAccent ? recess : theme.colors.line;
  const hairline = onAccent ? recess : theme.colors.lineSoft;
  // A tick is drawn *on* the box's own fill, which is the accent colour in this
  // tone and the plain accent otherwise.
  const onBoxFill = onAccent ? theme.colors.accent : theme.colors.surface;
  return StyleSheet.create({
  // The last block of a message drops its bottom margin — the surrounding
  // bubble/segment layout owns outer spacing.
  noBottom: { marginBottom: 0 },
  constrained: { minWidth: 0, maxWidth: "100%", alignSelf: "stretch" },
  previewList: { flex: 1, minWidth: 0, width: "100%" },
  // The preview's gutter belongs to the scrolling content: padding on a static
  // parent instead leaves a blank strip under the header once text scrolls.
  previewContent: { padding: spacing.lg, minWidth: 0, maxWidth: "100%", alignSelf: "stretch" },
  blockSpacing: { marginBottom: spacing.sm },
  bodyText: { color: text, ...theme.chat },
  paragraph: { color: text, ...theme.chat, marginBottom: spacing.sm },
  heading: {
    color: textStrong,
    fontWeight: "700",
  },
  // Heading levels 1-6, in the order `headingSizes` builds them.
  h1: { fontSize: theme.font(24), lineHeight: theme.leading(32) },
  h2: { fontSize: theme.font(21), lineHeight: theme.leading(29) },
  h3: { fontSize: theme.font(19), lineHeight: theme.leading(27) },
  h4: { fontSize: theme.font(17), lineHeight: theme.leading(25) },
  h5: { fontSize: theme.font(16), lineHeight: theme.leading(24) },
  h6: { fontSize: theme.font(15), lineHeight: theme.leading(22) },
  bold: { fontWeight: "700" },
  italic: { fontStyle: "italic" },
  strike: { textDecorationLine: "line-through" },
  inlineCode: {
    color: text,
    backgroundColor: recess,
    fontFamily: monospace,
    fontSize: theme.font(13),
    paddingHorizontal: 4,
    paddingVertical: 1,
    borderRadius: radius.sm,
    overflow: "hidden",
  },
  link: { color: accent, textDecorationLine: "underline" },
  // A conversation reference keeps the composer's `#` marker (see `SessionChip`).
  sessionChip: { color: accent, fontWeight: "600" },
  fileChip: {
    color: accent,
    backgroundColor: recess,
    borderRadius: radius.sm,
    paddingHorizontal: 5,
    paddingVertical: 2,
    overflow: "hidden",
  },
  rule: { height: 1, marginVertical: spacing.md, backgroundColor: line },
  codeContainer: {
    maxWidth: "100%",
    borderRadius: radius.md,
    overflow: "hidden",
    backgroundColor: recess,
  },
  codeToolbar: { flexDirection: "row", alignItems: "center", justifyContent: "space-between" },
  codeCopy: { padding: spacing.md, minHeight: 44, justifyContent: "center" },
  codeToggle: { padding: spacing.sm, minHeight: 44, alignItems: "center", justifyContent: "center", borderTopWidth: 1, borderTopColor: hairline },
  codeLanguage: { color: textMuted, fontSize: theme.font(12), paddingHorizontal: spacing.md, paddingTop: spacing.sm },
  code: {
    padding: spacing.md,
    color: text,
    fontFamily: monospace,
    fontSize: theme.font(13),
    lineHeight: theme.leading(20),
  },
  quote: {
    marginBottom: spacing.md,
    paddingLeft: spacing.md,
    borderLeftWidth: 3,
    borderLeftColor: accent,
  },
  list: { marginBottom: spacing.md },
  listRow: { flexDirection: "row", alignItems: "flex-start", marginBottom: spacing.xs },
  listBullet: {
    minWidth: 22,
    flexShrink: 0,
    textAlign: "right",
    marginRight: spacing.xs,
    color: text,
    ...theme.chat,
  },
  listItemBody: { flex: 1, minWidth: 0 },
  listItemText: { color: text, ...theme.chat },
  nestedBlocks: { marginTop: spacing.xs },
  checkbox: {
    width: 18,
    height: 18,
    marginTop: 4,
    marginRight: spacing.sm,
    alignItems: "center",
    justifyContent: "center",
    borderWidth: 1,
    borderColor: textMuted,
    borderRadius: radius.sm,
  },
  checkboxChecked: { backgroundColor: accent, borderColor: accent },
  checkMark: { color: onBoxFill, fontSize: theme.font(12), fontWeight: "700", lineHeight: theme.leading(14) },
  table: {
    borderWidth: 1,
    borderColor: hairline,
    borderRadius: radius.md,
    overflow: "hidden",
  },
  tableRow: { flexDirection: "row" },
  tableHead: { backgroundColor: recess },
  tableBodyRow: { borderTopWidth: 1, borderTopColor: hairline },
  tableRowZebra: { backgroundColor: recess },
  tableRowsToggle: { minHeight: 44, justifyContent: "center", paddingHorizontal: spacing.sm },
  tableRowsHint: { color: accent, fontSize: theme.font(12) },
  tableCell: { paddingHorizontal: spacing.sm, paddingVertical: spacing.sm },
  th: {
    color: textStrong,
    fontWeight: "700",
    fontSize: theme.font(14),
    lineHeight: theme.leading(20),
  },
  td: {
    color: textSoft,
    fontSize: theme.font(14),
    lineHeight: theme.leading(20),
  },
  cellBorderLeft: { borderLeftWidth: 1, borderLeftColor: hairline },
  });
}
