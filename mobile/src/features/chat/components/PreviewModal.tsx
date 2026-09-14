import { Download, ExternalLink, Share2, X } from "lucide-react-native";
import {
  ActivityIndicator,
  Image,
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
  return (
    <Modal
      animationType="slide"
      onDismiss={flushPendingPreviewAction}
      onRequestClose={closePreview}
      presentationStyle="pageSheet"
      visible={preview !== null}
    >
      <SafeAreaView style={styles.previewSafe}>
        <View style={styles.previewHeader}>
          <Text numberOfLines={1} style={styles.previewTitle}>
            {preview?.info.name}
          </Text>
          <Pressable
            accessibilityLabel={t("attachment.share")}
            accessibilityRole="button"
            style={({ pressed }) => [styles.iconButton, pressed && styles.pressed]}
            disabled={activeDownload !== null}
            onPress={() => {
              if (preview) {
                const attachment = preview.attachment;
                dismissPreviewThen(() => void downloadOriginal(attachment, "share"));
              }
            }}
          >
            <Share2 color={colors.ink} size={21} />
          </Pressable>
          <Pressable
            accessibilityLabel={t("attachment.open")}
            accessibilityRole="button"
            style={({ pressed }) => [styles.iconButton, pressed && styles.pressed]}
            disabled={activeDownload !== null}
            onPress={() => {
              if (preview) {
                const attachment = preview.attachment;
                dismissPreviewThen(() => void downloadOriginal(attachment, "open"));
              }
            }}
          >
            <ExternalLink color={colors.ink} size={21} />
          </Pressable>
          <Pressable
            accessibilityLabel={t("attachment.save")}
            accessibilityRole="button"
            style={({ pressed }) => [styles.iconButton, pressed && styles.pressed]}
            disabled={activeDownload !== null}
            onPress={() => {
              if (preview) {
                const attachment = preview.attachment;
                dismissPreviewThen(() => void downloadOriginal(attachment));
              }
            }}
          >
            {activeDownload !== null ? (
              <ActivityIndicator color={colors.ink} size="small" />
            ) : (
              <Download color={colors.ink} size={21} />
            )}
          </Pressable>
          <Pressable accessibilityRole="button" accessibilityLabel={t("common.close")} onPress={closePreview} style={({ pressed }) => [styles.iconButton, pressed && styles.pressed]}>
            <X color={colors.ink} size={22} />
          </Pressable>
        </View>
        {preview?.info.previewKind === "image" ? (
          <Image resizeMode="contain" source={{ uri: preview.uri }} style={styles.previewImage} />
        ) : preview?.info.previewKind === "markdown" ? (
          <ScrollView contentContainerStyle={styles.previewMarkdown}>
            {!!preview?.truncated && (
              <Text style={styles.previewTruncated}>{t("attachment.markdownTruncated")}</Text>
            )}
            <MarkdownText mode="file-preview" imageBasePath={preview?.attachment.path} text={preview?.markdown ?? ""} />
          </ScrollView>
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
      </SafeAreaView>
    </Modal>
  );
}

const styles = StyleSheet.create({
  previewSafe: { flex: 1, backgroundColor: colors.surface },
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
  previewTitle: { flex: 1, minWidth: 0, color: colors.inkStrong, fontSize: 16, fontWeight: "700" },
  previewImage: { flex: 1, width: "100%", height: "100%", backgroundColor: colors.surfaceSubtle },
  previewMarkdown: { padding: spacing.lg },
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
