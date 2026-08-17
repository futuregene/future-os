import { StyleSheet, Text, View } from "react-native";
import { useTranslation } from "react-i18next";
import type { ConnectionPhase } from "../remote/types";
import { colors, radius, spacing } from "../theme/tokens";

export function ConnectionBadge({
  phase,
  desktopOnline,
}: {
  phase: ConnectionPhase;
  desktopOnline: boolean;
}) {
  const { t } = useTranslation();
  const revoked = phase === "revoked";
  const failed = phase === "failed";
  const connected = phase === "connected" && desktopOnline;
  const reconnecting = phase === "connecting" || phase === "reconnecting" || phase === "refreshing";
  const label = revoked
    ? t("connection.revoked")
    : failed
      ? t("connection.failed")
      : connected
        ? t("connection.connected")
        : reconnecting
          ? t("connection.reconnecting")
          : t("connection.offline");

  return (
    <View
      style={[
        styles.badge,
        connected ? styles.connected : reconnecting ? styles.reconnecting : styles.offline,
      ]}
    >
      <View
        style={[
          styles.dot,
          connected
            ? styles.connectedDot
            : reconnecting
              ? styles.reconnectingDot
              : styles.offlineDot,
        ]}
      />
      <Text
        style={[
          styles.label,
          connected
            ? styles.connectedLabel
            : reconnecting
              ? styles.reconnectingLabel
              : styles.offlineLabel,
        ]}
      >
        {label}
      </Text>
    </View>
  );
}

const styles = StyleSheet.create({
  badge: {
    flexDirection: "row",
    alignItems: "center",
    gap: 6,
    borderWidth: 1,
    borderRadius: radius.pill,
    paddingHorizontal: spacing.md,
    paddingVertical: 6,
  },
  connected: { backgroundColor: colors.successSoft, borderColor: colors.successLine },
  reconnecting: { backgroundColor: colors.warningSoft, borderColor: colors.warningLine },
  offline: { backgroundColor: colors.dangerSoft, borderColor: colors.dangerLine },
  dot: { width: 7, height: 7, borderRadius: 4 },
  connectedDot: { backgroundColor: colors.success },
  reconnectingDot: { backgroundColor: colors.warning },
  offlineDot: { backgroundColor: colors.danger },
  label: { fontSize: 12, fontWeight: "600" },
  connectedLabel: { color: colors.success },
  reconnectingLabel: { color: colors.warning },
  offlineLabel: { color: colors.danger },
});
