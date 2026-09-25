import { useMemo, useState } from "react";
import { ChevronLeft, Download, Ellipsis, ExternalLink, Share2, X } from "lucide-react-native";
import {
  ActivityIndicator,
  FlatList,
  Modal,
  Platform,
  Pressable,
  ScrollView,
  StyleSheet,
  Text,
  View,
} from "react-native";
import type { TFunction } from "i18next";
import { SafeAreaView } from "react-native-safe-area-context";
import { CodeTokens } from "../../../components/CodeTokens";
import { codePreviewRows, codeRowText } from "../../../components/codePreviewRows";
import { codeLanguageForFile, codeTokenRows, highlightCode } from "../../../components/codeHighlight";
import { MarkdownText } from "../../../components/MarkdownText";
import { JsonPreview } from "../../../components/JsonPreview";
import type { HistoryAttachment } from "../../../remote/types";
import { colors, layout, radius, spacing } from "../../../theme/tokens";
import type { PreviewState } from "../useFileDownload";
import type { ActiveDownload, FileOperation } from "../utils";
import { ZoomableImage } from "./ZoomableImage";

/**
 * Identity for one layer of the preview stack. A changed key remounts the
 * layer, so it has to distinguish two documents, and two visits to the same
 * document (going A → B → A must not reuse B's layer, and the A underneath must
 * keep its own). Layers below the top stay mounted precisely so that their
 * scroll position survives; that only works while their key is stable.
 */
export function previewLayerKey(previews: PreviewState[], index: number): string {
  const path = previews[index]?.attachment.path ?? "";
  let visit = 0;
  for (let i = 0; i <= index; i += 1) {
    if (previews[i]?.attachment.path === path) visit += 1;
  }
  return `${path}#${visit}`;
}

export function PreviewModal({
  previews,
  activeDownload,
  closePreview,
  popPreview,
  dismissPreviewThen,
  downloadOriginal,
  flushPendingPreviewAction,
  openLinkedFile,
  t,
}: {
  previews: PreviewState[];
  activeDownload: ActiveDownload | null;
  closePreview: () => void;
  popPreview: () => void;
  dismissPreviewThen: (action: () => void) => void;
  downloadOriginal: (attachment: HistoryAttachment, operation?: FileOperation) => Promise<void>;
  flushPendingPreviewAction: () => void;
  openLinkedFile: (path: string) => Promise<void>;
  t: TFunction;
}) {
  const top = previews.length - 1;
  // The expanded menu is owned here rather than inside the layer that opened
  // it: every event that has to close it (a download, the stack changing, the
  // reader being dismissed natively) lands on this component, and none of them
  // reaches into a layer. Recording the stack it was opened against is what
  // makes "the documents changed" mean the same thing it did before the stack.
  const [menu, setMenu] = useState<{ key: string; stack: PreviewState[] } | null>(null);
  if (menu && (menu.stack !== previews || activeDownload !== null)) setMenu(null);
  const closeMenu = () => setMenu(null);
  return (
    <Modal
      animationType="slide"
      // `onDismiss` is iOS-only; it is the handoff point for an action that
      // needed this surface gone before it could be presented.
      onDismiss={() => {
        closeMenu();
        flushPendingPreviewAction();
      }}
      // Android's hardware back closes the menu first, then leaves one document
      // at a time, and closes the reader at the outermost one.
      onRequestClose={() => {
        if (menu) closeMenu();
        else if (previews.length > 1) popPreview();
        else closePreview();
      }}
      presentationStyle="pageSheet"
      visible={previews.length > 0}
    >
      <SafeAreaView style={styles.previewSafe}>
        {previews.map((preview, index) => {
          const key = previewLayerKey(previews, index);
          return (
            <PreviewLayer
              key={key}
              active={index === top}
              busy={activeDownload !== null}
              canGoBack={index === top && index > 0}
              closePreview={closePreview}
              closeMenu={closeMenu}
              dismissPreviewThen={dismissPreviewThen}
              downloadOriginal={downloadOriginal}
              menuOpen={menu?.key === key}
              onLinkedFile={openLinkedFile}
              openMenu={() => setMenu({ key, stack: previews })}
              popPreview={popPreview}
              preview={preview}
              t={t}
            />
          );
        })}
      </SafeAreaView>
    </Modal>
  );
}

/** One document of the stack: header, body, and its own overflow menu. A layer
 * under the top one stays mounted and untouched, which is what makes going back
 * return to the same place in the same document. */
function PreviewLayer({
  preview,
  active,
  busy,
  canGoBack,
  closePreview,
  closeMenu,
  downloadOriginal,
  dismissPreviewThen,
  menuOpen,
  onLinkedFile,
  openMenu,
  popPreview,
  t,
}: {
  preview: PreviewState;
  active: boolean;
  busy: boolean;
  canGoBack: boolean;
  closePreview: () => void;
  closeMenu: () => void;
  downloadOriginal: (attachment: HistoryAttachment, operation?: FileOperation) => Promise<void>;
  dismissPreviewThen: (action: () => void) => void;
  menuOpen: boolean;
  onLinkedFile: (path: string) => Promise<void>;
  openMenu: () => void;
  popPreview: () => void;
  t: TFunction;
}) {
  const [headerHeight, setHeaderHeight] = useState(60);
  // Highlight here rather than in `useFileDownload`: the plain-text route serves
  // both code files and prose (`.txt`, `.log`), so only the file name knows
  // whether this is source. Tokenizing is bounded — `highlightCode` refuses
  // oversized files and grammars it does not ship — and the fallback is the
  // untouched source, never mangled text. Recognized code keeps the monospace
  // metrics even when it is too large to color.
  const fileName = preview.attachment.name ?? "";
  const previewText = preview.text;
  const language = useMemo(() => codeLanguageForFile(fileName), [fileName]);
  const tokens = useMemo(
    () => (previewText === undefined ? null : highlightCode(previewText, language ?? undefined)),
    [language, previewText],
  );
  // The body is paged into bounded chunks whether or not there is a grammar.
  // A single `<Text>` holding the whole file is the one thing that cannot scale
  // here: the route serves up to 2 MiB, and highlighting can turn a 30 KB file
  // into thousands of nested spans. Native text layout pays for every mounted
  // span, so the document opens as a virtualized list of chunks — the same
  // bounding the chat's code blocks already use (`codePreviewRows`), just
  // without their collapse, since a preview is meant to be read in full.
  const rows = useMemo(() => codePreviewRows(previewText ?? ""), [previewText]);
  const rowTokens = useMemo(() => codeTokenRows(tokens, rows), [tokens, rows]);
  // A layer that got covered lost its menu with its focus: it belonged to the
  // document the reader was looking at.
  const shown = menuOpen && active;
  const close = () => {
    closeMenu();
    closePreview();
  };
  const selectAction = (operation: FileOperation) => {
    if (busy) return;
    const attachment = preview.attachment;
    closeMenu();
    dismissPreviewThen(() => {
      if (operation === "save") void downloadOriginal(attachment);
      else void downloadOriginal(attachment, operation);
    });
  };

  return (
    <View
      accessibilityElementsHidden={!active}
      importantForAccessibility={active ? "auto" : "no-hide-descendants"}
      pointerEvents={active ? "auto" : "none"}
      style={styles.previewLayer}
      testID="preview-layer"
    >
      <View style={styles.previewBody}>
        <View
          style={styles.previewBody}
          accessibilityElementsHidden={shown}
          importantForAccessibility={shown ? "no-hide-descendants" : "auto"}
        >
          <View
            style={styles.previewHeader}
            onLayout={event => {
              if (active) setHeaderHeight(event.nativeEvent.layout.height);
            }}
          >
            {canGoBack ? (
              <Pressable
                accessibilityLabel={t("common.back")}
                accessibilityRole="button"
                onPress={popPreview}
                style={({ pressed }) => [styles.iconButton, pressed && styles.pressed]}
              >
                <ChevronLeft color={colors.ink} size={22} />
              </Pressable>
            ) : null}
            <Text numberOfLines={1} style={styles.previewTitle}>
              {preview.info.name}
            </Text>
            <Pressable
              accessibilityLabel={t("common.more")}
              accessibilityRole="button"
              accessibilityState={{ expanded: shown, disabled: busy, busy }}
              style={({ pressed }) => [styles.iconButton, pressed && styles.pressed, busy && styles.disabled]}
              disabled={busy}
              onPress={openMenu}
            >
              {busy ? (
                <ActivityIndicator color={colors.ink} size="small" />
              ) : (
                <Ellipsis color={colors.ink} size={21} />
              )}
            </Pressable>
            <Pressable accessibilityRole="button" accessibilityLabel={t("common.close")} onPress={close} style={({ pressed }) => [styles.iconButton, pressed && styles.pressed]}>
              <X color={colors.ink} size={22} />
            </Pressable>
          </View>
          {preview.info.previewKind === "image" ? (
            // Pinch to zoom / drag to pan: a phone-sized preview of a screenshot is
            // unreadable without it.
            <ZoomableImage key={preview.uri} accessibilityLabel={preview.info.name} uri={preview.uri} />
          ) : preview.info.previewKind === "markdown" ? (
            <View style={styles.previewDocument}>
              {!!preview.truncated && (
                <View style={styles.previewNotice}>
                  <Text style={styles.previewTruncated}>{t("attachment.markdownTruncated")}</Text>
                </View>
              )}
              <MarkdownText
                imageBasePath={preview.attachment.path}
                mode="file-preview"
                onOpenFile={path => void onLinkedFile(path)}
                text={preview.markdown ?? ""}
              />
            </View>
          ) : preview.info.previewKind === "json" ? (
            <JsonPreview
              invalidMessage={detail => t("attachment.jsonInvalid", { detail })}
              sourceTruncated={!!preview.truncated}
              text={preview.text ?? ""}
              tooComplexMessage={t("attachment.jsonTooComplex")}
              truncatedMessage={t("attachment.jsonTruncated")}
            />
          ) : (
            <FlatList
              contentContainerStyle={styles.previewMarkdown}
              data={rows}
              initialNumToRender={12}
              keyExtractor={(_row, index) => String(index)}
              ListHeaderComponent={preview.truncated
                ? <Text style={styles.previewTruncated}>{t("attachment.textTruncated")}</Text>
                : null}
              maxToRenderPerBatch={12}
              renderItem={({ item, index }) => (
                <Text selectable style={language ? styles.previewCode : styles.previewText}>
                  <CodeTokens fallback={codeRowText(item.text)} tokens={rowTokens[index] ?? null} />
                </Text>
              )}
              windowSize={5}
            />
          )}
        </View>
        {shown && (
          // Stay inside the preview's native Modal. A second Modal would compete
          // with the preview-to-system-share/open presentation handoff on iOS.
          <View style={[styles.menuOverlay, { paddingTop: headerHeight + spacing.xs }]}>
            <Pressable
              accessible={false}
              testID="preview-menu-backdrop"
              style={StyleSheet.absoluteFill}
              onPress={closeMenu}
            />
            <View accessibilityViewIsModal onAccessibilityEscape={closeMenu} style={styles.menu}>
              <ScrollView bounces={false} contentContainerStyle={styles.menuContent}>
                {([
                  { operation: "share", Icon: Share2 },
                  { operation: "open", Icon: ExternalLink },
                  { operation: "save", Icon: Download },
                ] as const).map(({ operation, Icon }) => (
                  <Pressable
                    key={operation}
                    accessibilityRole="button"
                    accessibilityLabel={t(`attachment.${operation}`)}
                    onPress={() => selectAction(operation)}
                    style={({ pressed }) => [styles.menuAction, pressed && styles.pressed]}
                  >
                    <Icon color={colors.ink} size={20} />
                    <Text style={styles.menuLabel}>{t(`attachment.${operation}`)}</Text>
                  </Pressable>
                ))}
              </ScrollView>
            </View>
          </View>
        )}
      </View>
    </View>
  );
}

// Matches the chat's code blocks (`MarkdownText`).
const monospace = Platform.select({ ios: "Menlo", default: "monospace" });

const styles = StyleSheet.create({
  previewSafe: { flex: 1, backgroundColor: colors.surface },
  // A layer fills the reader so the layers below stay laid out, and therefore
  // keep their scroll offsets. Later siblings paint on top.
  previewLayer: { ...StyleSheet.absoluteFill },
  previewBody: { flex: 1, minHeight: 0 },
  previewHeader: {
    minHeight: 60,
    flexDirection: "row",
    alignItems: "center",
    gap: spacing.xs,
    paddingHorizontal: spacing.sm,
    borderBottomWidth: 1,
    borderBottomColor: colors.lineSoft,
  },
  iconButton: { width: layout.touchTarget, height: layout.touchTarget, alignItems: "center", justifyContent: "center", borderRadius: radius.md },
  pressed: { backgroundColor: colors.surfaceSubtle },
  disabled: { opacity: 0.4 },
  previewTitle: { flex: 1, minWidth: 0, color: colors.inkStrong, fontSize: 16, fontWeight: "700" },
  menuOverlay: { ...StyleSheet.absoluteFill, alignItems: "flex-end", padding: spacing.sm },
  menu: {
    width: "100%",
    maxWidth: 240,
    flexShrink: 1,
    borderRadius: radius.md,
    borderWidth: 1,
    borderColor: colors.line,
    backgroundColor: colors.surface,
    boxShadow: "0 4px 16px rgba(15, 23, 42, 0.15)",
    overflow: "hidden",
  },
  menuContent: { padding: spacing.xs },
  menuAction: { minHeight: layout.touchTarget, flexDirection: "row", alignItems: "center", gap: spacing.md, paddingHorizontal: spacing.md, paddingVertical: spacing.sm, borderRadius: radius.sm },
  menuLabel: { flexShrink: 1, color: colors.ink, fontSize: 15 },
  previewMarkdown: { padding: spacing.lg },
  // No padding here: the document itself scrolls, so its gutter lives in the
  // markdown list's content container. Padding on this static parent instead
  // leaves a blank strip under the header, where the list clips scrolled text.
  // Only the notice above the list sits outside that scrolling surface.
  previewDocument: { flex: 1, minHeight: 0 },
  previewNotice: { paddingHorizontal: spacing.lg, paddingTop: spacing.lg },
  previewTruncated: {
    marginBottom: spacing.md,
    padding: spacing.md,
    borderRadius: radius.md,
    color: colors.warning,
    backgroundColor: colors.warningSoft,
    fontSize: 12,
  },
  previewText: { color: colors.ink, fontSize: 14, lineHeight: 21 },
  // Code files get the monospace metrics the chat's code blocks use, so columns
  // line up in the colored spans; prose (`.txt`, `.log`) keeps the proportional
  // `previewText` it had before.
  previewCode: { color: colors.ink, fontFamily: monospace, fontSize: 13, lineHeight: 20 },
});
