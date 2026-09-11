import { Pressable, StyleSheet, Text, View } from "react-native";
import { useTranslation } from "react-i18next";
import type { ConnectionPresentation } from "../remote/connectionPresentation";
import { colors, radius, spacing } from "../theme/tokens";

export function ConnectionBadge({
  presentation,
  onReconnect,
  compact = false,
}: {
  presentation: ConnectionPresentation;
  onReconnect?: () => void;
  compact?: boolean;
}) {
  const { t } = useTranslation();
  const connected = presentation.level === "connected";
  const connecting = presentation.level === "connecting";
  const reconnectable =
    presentation.action === "reconnect" ||
    presentation.action === "retry" ||
    presentation.action === "checkNetwork";
  const label =
    t(presentation.titleKey) + (presentation.supportCode ? ` (${presentation.supportCode})` : "");

  return (
    <Pressable
      accessibilityLabel={reconnectable ? `${label}. ${t("connection.reconnect")}` : label}
      accessibilityRole={reconnectable ? "button" : undefined}
      disabled={!reconnectable}
      onPress={reconnectable ? onReconnect : undefined}
      style={[
        styles.badge,
        connected ? styles.connected : connecting ? styles.connecting : styles.disconnected,
        compact && styles.compact,
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
      {!compact && (
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
      )}
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
  compact: {
    width: 44,
    minHeight: 44,
    justifyContent: "center",
    paddingHorizontal: 0,
    borderColor: colors.lineSoft,
  },
  connected: { backgroundColor: colors.infoSoft, borderColor: colors.infoLine },
  connecting: { backgroundColor: colors.warningSoft, borderColor: colors.warningLine },
  disconnected: { backgroundColor: colors.dangerSoft, borderColor: colors.dangerLine },
  dot: { width: 7, height: 7, borderRadius: 4 },
  connectedDot: { backgroundColor: colors.info },
  connectingDot: { backgroundColor: colors.warning },
  disconnectedDot: { backgroundColor: colors.danger },
  label: { fontSize: 12, fontWeight: "600" },
  connectedLabel: { color: colors.info },
  connectingLabel: { color: colors.warning },
  disconnectedLabel: { color: colors.danger },
});
