import { Unplug } from "lucide-react-native";
import { useTranslation } from "react-i18next";
import { ActivityIndicator, StyleSheet, Text, View } from "react-native";
import { Button } from "../components/Button";
import { useRemote } from "../remote/RemoteContext";
import { disconnectedCopy } from "../remote/disconnectedCopy";
import { spacing } from "../theme/tokens";
import { createThemedStyles, useTheme } from "../theme/ThemeContext";

export function DisconnectedScreen({
  reconnecting = false,
  onReconnect,
  onUnpair,
}: {
  reconnecting?: boolean;
  onReconnect(): void;
  onUnpair(): void;
}) {
  const styles = useStyles();
  const { colors } = useTheme();
  const { t } = useTranslation();
  const remote = useRemote();
  const presentation = remote.connectionPresentation;
  const copy = disconnectedCopy(presentation, remote.presence?.reason, remote.error);
  const pairingExpired = !reconnecting && presentation.customerState === "pairingExpired";
  return (
    <View style={styles.page}>
      <View style={styles.content}>
        <View style={styles.iconSlot}>
          {reconnecting ? (
            <ActivityIndicator color={colors.warning} size="large" />
          ) : (
            <Unplug color={colors.danger} size={40} />
          )}
        </View>
        <Text
          accessibilityLiveRegion="polite"
          style={[styles.title, reconnecting && styles.reconnectingTitle]}
        >
          {t(reconnecting
            ? "connection.connecting"
            : pairingExpired
              ? "connection.pairingExpired"
              : "connection.disconnected")}
        </Text>
        <View style={styles.details}>
          {reconnecting ? (
            <>
              <Text style={styles.hint}>{t("connection.statusDetails.connecting.reason")}</Text>
              <Text style={styles.hint}>{t("connection.statusDetails.connecting.solution")}</Text>
            </>
          ) : (
            <>
            <Text style={styles.hint}>
              {t(`connection.disconnectDetails.${copy}.reason`)}
              {presentation.supportCode ? ` (${presentation.supportCode})` : ""}
            </Text>
            <Text style={styles.hint}>{t(`connection.disconnectDetails.${copy}.solution`)}</Text>
            </>
          )}
        </View>
        <View style={styles.actionSlot}>
          <Button
            label={t(pairingExpired ? "sessions.unpair" : "connection.reconnect")}
            loading={reconnecting}
            onPress={pairingExpired ? onUnpair : onReconnect}
          />
        </View>
      </View>
    </View>
  );
}

const useStyles = createThemedStyles(theme => StyleSheet.create({
  page: { flex: 1, backgroundColor: theme.colors.surface, justifyContent: "center" },
  content: { alignItems: "center", padding: spacing.xl, gap: spacing.md },
  iconSlot: { height: 40, alignItems: "center", justifyContent: "center" },
  title: { color: theme.colors.danger, fontSize: theme.font(18), fontWeight: "600" },
  reconnectingTitle: { color: theme.colors.warning },
  details: {
    alignSelf: "stretch",
    minHeight: 42,
    alignItems: "center",
    justifyContent: "center",
    gap: spacing.xs,
  },
  actionSlot: { minHeight: 48, alignItems: "center", justifyContent: "center" },
  hint: { color: theme.colors.inkMuted, textAlign: "center", fontSize: theme.font(14) },
}));
