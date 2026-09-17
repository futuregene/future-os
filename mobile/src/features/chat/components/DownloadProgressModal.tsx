import { Modal, Pressable, StyleSheet, Text, View } from "react-native";
import type { TFunction } from "i18next";
import { colors, layout, radius, spacing } from "../../../theme/tokens";
import { DialogSurface } from "../../../components/DialogSurface";
import { formatBytes, type ActiveDownload } from "../utils";

export function DownloadProgressModal({
  activeDownload,
  activeDownloadFraction,
  cancelActiveDownload,
  flushPendingDownloadModal,
  onDownloadModalShow,
  t,
}: {
  activeDownload: ActiveDownload | null;
  activeDownloadFraction: number;
  cancelActiveDownload: () => void;
  flushPendingDownloadModal: () => void;
  onDownloadModalShow: () => void;
  t: TFunction;
}) {
  const cancelling = activeDownload?.phase === "cancelling";
  return (
    <Modal
      animationType="fade"
      onDismiss={flushPendingDownloadModal}
      onRequestClose={cancelActiveDownload}
      onShow={onDownloadModalShow}
      transparent
      visible={activeDownload !== null}
    >
      <DialogSurface>
        <View style={styles.header}>
          <Text numberOfLines={1} accessibilityRole="header" style={styles.downloadTitle}>{t("attachment.downloadProgressTitle")}</Text>
          <Pressable
            accessibilityRole="button"
            accessibilityLabel={t("chat.cancel")}
            accessibilityState={{ disabled: cancelling }}
            disabled={cancelling}
            onPress={cancelActiveDownload}
            style={({ pressed }) => [styles.downloadCancel, pressed && styles.downloadCancelPressed, cancelling && styles.disabled]}
          >
            <Text style={styles.downloadCancelText}>{t("chat.cancel")}</Text>
          </Pressable>
        </View>
        <View style={styles.identity}>
          <Text numberOfLines={1} ellipsizeMode="middle" style={styles.downloadFileName}>
            {activeDownload?.fileName}
          </Text>
          <Text numberOfLines={1} style={styles.downloadPhase}>
            {activeDownload ? t(`attachment.downloadPhases.${activeDownload.phase}`) : ""}
          </Text>
        </View>
        <View style={styles.progress}>
          <View style={styles.downloadTrack}>
            <View
              style={[
                styles.downloadFill,
                { width: `${Math.max(2, activeDownloadFraction * 100)}%` },
              ]}
            />
          </View>
          <View style={styles.downloadMeta}>
            <Text numberOfLines={1} ellipsizeMode="middle" style={styles.downloadBytes}>
              {activeDownload?.totalBytes
                ? `${activeDownload.completedBytes === 0 ? "0 KB" : formatBytes(activeDownload.completedBytes)} / ${formatBytes(activeDownload.totalBytes)}`
                : t("attachment.calculatingSize")}
            </Text>
            <Text style={styles.downloadPercent}>
              {activeDownload?.totalBytes ? `${Math.round(activeDownloadFraction * 100)}%` : ""}
            </Text>
          </View>
        </View>
      </DialogSurface>
    </Modal>
  );
}

const styles = StyleSheet.create({
  header: { flexDirection: "row", alignItems: "center", gap: spacing.sm },
  downloadTitle: { flex: 1, minWidth: 0, color: colors.inkStrong, fontSize: 20, fontWeight: "700" },
  identity: { gap: spacing.xs },
  progress: { gap: spacing.sm },
  downloadFileName: { color: colors.ink, fontSize: 14, fontWeight: "600" },
  downloadPhase: { color: colors.inkMuted, fontSize: 13 },
  downloadTrack: {
    height: 7,
    overflow: "hidden",
    borderRadius: radius.pill,
    backgroundColor: colors.surfaceSubtle,
  },
  downloadFill: { height: 7, borderRadius: radius.pill, backgroundColor: colors.accent },
  downloadMeta: { flexDirection: "row", alignItems: "center", gap: spacing.sm },
  downloadBytes: { flex: 1, minWidth: 0, color: colors.inkMuted, fontSize: 12 },
  downloadPercent: { flexShrink: 0, color: colors.inkSoft, fontSize: 12, fontWeight: "600" },
  downloadCancel: {
    minHeight: layout.touchTarget,
    minWidth: layout.touchTarget,
    justifyContent: "center",
    paddingHorizontal: spacing.sm,
    paddingVertical: spacing.sm,
    borderRadius: radius.md,
  },
  downloadCancelPressed: { backgroundColor: colors.surfaceSubtle },
  disabled: { opacity: 0.5 },
  downloadCancelText: { color: colors.accent, fontSize: 14, fontWeight: "600" },
});
