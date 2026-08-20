import { Pressable, StyleSheet, Text, View } from "react-native";
import { useTranslation } from "react-i18next";
import type { ConnectionPhase } from "../remote/types";
import { colors, radius, spacing } from "../theme/tokens";

export function ConnectionBadge({
  phase,
  desktopOnline,
  onReconnect,
}: {
  phase: ConnectionPhase;
  desktopOnline: boolean;
  onReconnect?: () => void;
}) {
  const { t } = useTranslation();
  const connected = phase === "ready" && desktopOnline;
  const connecting = phase === "connecting" || phase === "reconnecting" || phase === "refreshing";
  const disconnected = !connected && !connecting;
  const label = connected
    ? t("connection.connected")
    : connecting
      ? t("connection.connecting")
      : t("connection.disconnected");

  return (
    <Pressable
      accessibilityLabel={disconnected ? t("connection.reconnect") : label}
      accessibilityRole={disconnected ? "button" : undefined}
      disabled={!disconnected}
      onPress={disconnected ? onReconnect : undefined}
      style={[
        styles.badge,
        connected ? styles.connected : connecting ? styles.connecting : styles.disconnected,
      ]}
    >
      <View
        style={[
          styles.dot,
          connected
            ? styles.connectedDot
            : connecting
              ? styles.connectingDot
              : styles.disconnectedDot,
        ]}
      />
      <Text
        style={[
          styles.label,
          connected
            ? styles.connectedLabel
            : connecting
              ? styles.connectingLabel
              : styles.disconnectedLabel,
        ]}
      >
        {label}
      </Text>
    </Pressable>
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
  connecting: { backgroundColor: colors.warningSoft, borderColor: colors.warningLine },
  disconnected: { backgroundColor: colors.dangerSoft, borderColor: colors.dangerLine },
  dot: { width: 7, height: 7, borderRadius: 4 },
  connectedDot: { backgroundColor: colors.success },
  connectingDot: { backgroundColor: colors.warning },
  disconnectedDot: { backgroundColor: colors.danger },
  label: { fontSize: 12, fontWeight: "600" },
  connectedLabel: { color: colors.success },
  connectingLabel: { color: colors.warning },
  disconnectedLabel: { color: colors.danger },
});
