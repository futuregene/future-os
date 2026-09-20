import { useState } from "react";
import { Download, Ellipsis, ExternalLink, Share2, X } from "lucide-react-native";
import {
  ActivityIndicator,
  Modal,
  Pressable,
  ScrollView,
  StyleSheet,
  Text,
  View,
} from "react-native";
import type { TFunction } from "i18next";
import { SafeAreaView } from "react-native-safe-area-context";
import { MarkdownText } from "../../../components/MarkdownText";
import { JsonPreview } from "../../../components/JsonPreview";
import type { HistoryAttachment } from "../../../remote/types";
import { colors, layout, radius, spacing } from "../../../theme/tokens";
import type { PreviewState } from "../useFileDownload";
import type { ActiveDownload, FileOperation } from "../utils";
import { ZoomableImage } from "./ZoomableImage";

export function PreviewModal({
  preview,
  activeDownload,
  closePreview,
  dismissPreviewThen,
  downloadOriginal,
  flushPendingPreviewAction,
  t,
}: {
  preview: PreviewState | null;
  activeDownload: ActiveDownload | null;
  closePreview: () => void;
  dismissPreviewThen: (action: () => void) => void;
  downloadOriginal: (attachment: HistoryAttachment, operation?: FileOperation) => Promise<void>;
  flushPendingPreviewAction: () => void;
  t: TFunction;
}) {
  const [menuFor, setMenuFor] = useState<PreviewState | null>(null);
  const [headerHeight, setHeaderHeight] = useState(60);
  // Do not carry an expanded menu into a new preview or an active download.
  if (menuFor && (menuFor !== preview || activeDownload !== null)) setMenuFor(null);
  const menuOpen = menuFor !== null && menuFor === preview && activeDownload === null;
  const close = () => {
    setMenuFor(null);
    closePreview();
  };
  const selectAction = (operation: FileOperation) => {
    if (!preview || activeDownload !== null) return;
    const attachment = preview.attachment;
    setMenuFor(null);
    dismissPreviewThen(() => {
      if (operation === "save") void downloadOriginal(attachment);
      else void downloadOriginal(attachment, operation);
    });
  };

  return (
    <Modal
      animationType="slide"
      onDismiss={() => { setMenuFor(null); flushPendingPreviewAction(); }}
      onRequestClose={() => { if (menuOpen) setMenuFor(null); else close(); }}
      presentationStyle="pageSheet"
      visible={preview !== null}
    >
      <SafeAreaView style={styles.previewSafe}>
        <View style={styles.previewBody}>
          <View
            style={styles.previewBody}
            accessibilityElementsHidden={menuOpen}
            importantForAccessibility={menuOpen ? "no-hide-descendants" : "auto"}
          >
            <View style={styles.previewHeader} onLayout={event => setHeaderHeight(event.nativeEvent.layout.height)}>
              <Text numberOfLines={1} style={styles.previewTitle}>
                {preview?.info.name}
              </Text>
              <Pressable
                accessibilityLabel={t("common.more")}
                accessibilityRole="button"
                accessibilityState={{ expanded: menuOpen, disabled: activeDownload !== null, busy: activeDownload !== null }}
                style={({ pressed }) => [styles.iconButton, pressed && styles.pressed, activeDownload !== null && styles.disabled]}
                disabled={activeDownload !== null}
                onPress={() => setMenuFor(preview)}
              >
                {activeDownload !== null ? (
                  <ActivityIndicator color={colors.ink} size="small" />
                ) : (
                  <Ellipsis color={colors.ink} size={21} />
                )}
              </Pressable>
              <Pressable accessibilityRole="button" accessibilityLabel={t("common.close")} onPress={close} style={({ pressed }) => [styles.iconButton, pressed && styles.pressed]}>
                <X color={colors.ink} size={22} />
              </Pressable>
            </View>
            {preview?.info.previewKind === "image" ? (
              // Pinch to zoom / drag to pan: a phone-sized preview of a screenshot is
              // unreadable without it.
              <ZoomableImage key={preview.uri} accessibilityLabel={preview.info.name} uri={preview.uri} />
            ) : preview?.info.previewKind === "markdown" ? (
              <View style={styles.previewDocument}>
                {!!preview?.truncated && (
                  <View style={styles.previewNotice}>
                    <Text style={styles.previewTruncated}>{t("attachment.markdownTruncated")}</Text>
                  </View>
                )}
                <MarkdownText mode="file-preview" imageBasePath={preview?.attachment.path} text={preview?.markdown ?? ""} />
              </View>
            ) : preview?.info.previewKind === "json" ? (
              <JsonPreview
                invalidMessage={detail => t("attachment.jsonInvalid", { detail })}
                sourceTruncated={!!preview.truncated}
                text={preview.text ?? ""}
                tooComplexMessage={t("attachment.jsonTooComplex")}
                truncatedMessage={t("attachment.jsonTruncated")}
              />
            ) : (
              <ScrollView contentContainerStyle={styles.previewMarkdown}>
                {!!preview?.truncated && (
                  <Text style={styles.previewTruncated}>{t("attachment.textTruncated")}</Text>
                )}
                <Text selectable style={styles.previewText}>
                  {preview?.text ?? ""}
                </Text>
              </ScrollView>
            )}
          </View>
          {menuOpen && (
            // Stay inside the preview's native Modal. A second Modal would compete
            // with the preview-to-system-share/open presentation handoff on iOS.
            <View style={[styles.menuOverlay, { paddingTop: headerHeight + spacing.xs }]}>
              <Pressable
                accessible={false}
                testID="preview-menu-backdrop"
                style={StyleSheet.absoluteFill}
                onPress={() => setMenuFor(null)}
              />
              <View accessibilityViewIsModal onAccessibilityEscape={() => setMenuFor(null)} style={styles.menu}>
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
      </SafeAreaView>
    </Modal>
  );
}

const styles = StyleSheet.create({
  previewSafe: { flex: 1, backgroundColor: colors.surface },
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
});
