import { Download, X } from "lucide-react-native";
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
import { colors, radius, spacing } from "../../../theme/tokens";
import type { PreviewState } from "../useFileDownload";
import type { ActiveDownload } from "../utils";

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
  downloadOriginal: (attachment: HistoryAttachment) => Promise<void>;
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
            accessibilityLabel={t("attachment.save")}
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
          <Pressable accessibilityLabel={t("common.close")} onPress={closePreview}>
            <X color={colors.ink} size={22} />
          </Pressable>
        </View>
        {preview?.info.previewKind === "image" ? (
          <Image
            resizeMode="contain"
            source={{ uri: preview.uri }}
            style={styles.previewImage}
          />
        ) : preview?.info.previewKind === "markdown" ? (
          <ScrollView contentContainerStyle={styles.previewMarkdown}>
            {!!preview?.truncated && (
              <Text style={styles.previewTruncated}>{t("attachment.markdownTruncated")}</Text>
            )}
            <MarkdownText mode="file-preview" text={preview?.markdown ?? ""} />
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
    minHeight: 52,
    flexDirection: "row",
    alignItems: "center",
    gap: spacing.md,
    paddingHorizontal: spacing.lg,
    borderBottomWidth: 1,
    borderBottomColor: colors.lineSoft,
  },
  previewTitle: { flex: 1, color: colors.inkStrong, fontSize: 16, fontWeight: "700" },
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
