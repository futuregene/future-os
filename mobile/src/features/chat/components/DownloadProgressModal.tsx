import { Modal, Pressable, StyleSheet, Text, View } from "react-native";
import type { TFunction } from "i18next";
import { colors, radius, spacing } from "../../../theme/tokens";
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
  return (
    <Modal
      animationType="fade"
      onDismiss={flushPendingDownloadModal}
      onRequestClose={cancelActiveDownload}
      onShow={onDownloadModalShow}
      transparent
      visible={activeDownload !== null}
    >
      <View style={styles.downloadOverlay}>
        <View style={styles.downloadDialog}>
          <Text style={styles.downloadTitle}>{t("attachment.downloadProgressTitle")}</Text>
          <Text numberOfLines={1} style={styles.downloadFileName}>
            {activeDownload?.fileName}
          </Text>
          <Text style={styles.downloadPhase}>
            {activeDownload ? t(`attachment.downloadPhases.${activeDownload.phase}`) : ""}
          </Text>
          <View style={styles.downloadTrack}>
            <View
              style={[
                styles.downloadFill,
                { width: `${Math.max(2, activeDownloadFraction * 100)}%` },
              ]}
            />
          </View>
          <View style={styles.downloadMeta}>
            <Text style={styles.downloadBytes}>
              {activeDownload?.totalBytes
                ? `${activeDownload.completedBytes === 0 ? "0 KB" : formatBytes(activeDownload.completedBytes)} / ${formatBytes(activeDownload.totalBytes)}`
                : t("attachment.calculatingSize")}
            </Text>
            <Text style={styles.downloadPercent}>
              {activeDownload?.totalBytes ? `${Math.round(activeDownloadFraction * 100)}%` : ""}
            </Text>
          </View>
          <Pressable
            disabled={activeDownload?.phase === "cancelling"}
            onPress={cancelActiveDownload}
            style={({ pressed }) => [
              styles.downloadCancel,
              pressed && styles.downloadCancelPressed,
            ]}
          >
            <Text style={styles.downloadCancelText}>{t("chat.cancel")}</Text>
          </Pressable>
        </View>
      </View>
    </Modal>
  );
}

const styles = StyleSheet.create({
  downloadOverlay: {
    flex: 1,
    alignItems: "center",
    justifyContent: "center",
    padding: spacing.xl,
    backgroundColor: colors.overlay,
  },
  downloadDialog: {
    width: "100%",
    maxWidth: 360,
    gap: spacing.md,
    padding: spacing.xl,
    borderRadius: radius.lg,
    backgroundColor: colors.surface,
  },
  downloadTitle: { color: colors.inkStrong, fontSize: 17, fontWeight: "700" },
  downloadFileName: { color: colors.ink, fontSize: 14, fontWeight: "600" },
  downloadPhase: { color: colors.inkMuted, fontSize: 13 },
  downloadTrack: {
    height: 7,
    overflow: "hidden",
    borderRadius: radius.pill,
    backgroundColor: colors.surfaceSubtle,
  },
  downloadFill: { height: 7, borderRadius: radius.pill, backgroundColor: colors.accent },
  downloadMeta: { flexDirection: "row", justifyContent: "space-between" },
  downloadBytes: { color: colors.inkMuted, fontSize: 12 },
  downloadPercent: { color: colors.inkSoft, fontSize: 12, fontWeight: "600" },
  downloadCancel: {
    alignSelf: "flex-end",
    paddingHorizontal: spacing.md,
    paddingVertical: spacing.sm,
    borderRadius: radius.md,
  },
  downloadCancelPressed: { backgroundColor: colors.surfaceSubtle },
  downloadCancelText: { color: colors.accent, fontSize: 14, fontWeight: "600" },
});
