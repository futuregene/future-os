import { Pressable, StyleSheet, Text, View } from "react-native";
import { useTranslation } from "react-i18next";
import type { ConnectionPresentation } from "../remote/connectionPresentation";
import { colors, layout, radius, spacing } from "../theme/tokens";

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
      style={({ pressed }) => [
        styles.badge,
        connected ? styles.connected : connecting ? styles.connecting : styles.disconnected,
        compact && styles.compact,
        pressed && reconnectable && styles.pressed,
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
          numberOfLines={1}
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
    minHeight: layout.touchTarget,
    maxWidth: 180,
    flexShrink: 0,
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
  pressed: { opacity: 0.7 },
  connected: { backgroundColor: colors.successSoft, borderColor: colors.successLine },
  connecting: { backgroundColor: colors.warningSoft, borderColor: colors.warningLine },
  disconnected: { backgroundColor: colors.dangerSoft, borderColor: colors.dangerLine },
  dot: { width: 7, height: 7, borderRadius: 4 },
  connectedDot: { backgroundColor: colors.success },
  connectingDot: { backgroundColor: colors.warning },
  disconnectedDot: { backgroundColor: colors.danger },
  label: { flexShrink: 1, fontSize: 12, fontWeight: "600" },
  connectedLabel: { color: colors.success },
  connectingLabel: { color: colors.warning },
  disconnectedLabel: { color: colors.danger },
});
