import { ChevronRight, LogOut, MessageSquarePlus, RefreshCw } from "lucide-react-native";
import { useTranslation } from "react-i18next";
import { Alert, FlatList, Pressable, SafeAreaView, StyleSheet, Text, View } from "react-native";
import { Button } from "../components/Button";
import { ConnectionBadge } from "../components/ConnectionBadge";
import { useRemote } from "../remote/RemoteContext";
import type { RemoteSession } from "../remote/types";
import { colors, radius, spacing } from "../theme/tokens";
import { VERSION } from "../version.generated";

export function SessionsScreen() {
  const { t } = useTranslation();
  const remote = useRemote();

  const confirmUnpair = () => {
    Alert.alert(t("sessions.unpair"), t("sessions.unpairConfirm"), [
      { text: t("chat.cancel"), style: "cancel" },
      { text: t("sessions.unpair"), style: "destructive", onPress: () => void remote.unpair() },
    ]);
  };

  const renderSession = ({ item }: { item: RemoteSession }) => (
    <Pressable
      accessibilityRole="button"
      onPress={() => void remote.selectSession(item.sessionId)}
      style={({ pressed }) => [styles.session, pressed && styles.pressed]}
    >
      <View style={styles.sessionCopy}>
        <Text numberOfLines={1} style={styles.sessionTitle}>
          {item.title || t("sessions.unnamed")}
        </Text>
        <Text numberOfLines={1} style={styles.sessionId}>
          {item.sessionId}
        </Text>
      </View>
      <ChevronRight color={colors.inkMuted} size={19} />
    </Pressable>
  );

  return (
    <SafeAreaView style={styles.safe}>
      <View style={styles.page}>
        <View style={styles.topbar}>
          <View>
            <Text style={styles.kicker}>{t("appName")}</Text>
            <Text style={styles.title}>{t("sessions.title")}</Text>
          </View>
          <ConnectionBadge phase={remote.phase} desktopOnline={remote.desktopOnline} />
        </View>

        {!remote.desktopOnline && (
          <View style={styles.offline}>
            <Text style={styles.offlineTitle}>{t("connection.offline")}</Text>
            <Text style={styles.offlineText}>{t("connection.offlineHint")}</Text>
            {remote.phase === "error" && (
              <Button
                compact
                label={t("connection.retry")}
                loading={remote.busy}
                onPress={() => void remote.reconnect()}
                variant="secondary"
              />
            )}
          </View>
        )}

        <Button
          disabled={!remote.desktopOnline}
          icon={<MessageSquarePlus color={colors.surface} size={19} />}
          label={t("sessions.new")}
          onPress={() => void remote.newConversation()}
        />

        <View style={styles.listHeader}>
          <Text style={styles.listTitle}>{t("sessions.title")}</Text>
          <Button
            compact
            icon={<RefreshCw color={colors.inkSoft} size={16} />}
            label={t("sessions.refresh")}
            onPress={() => void remote.refreshSessions()}
            variant="ghost"
          />
        </View>

        <FlatList
          contentContainerStyle={remote.sessions.length === 0 ? styles.emptyList : styles.list}
          data={remote.sessions}
          keyExtractor={item => item.sessionId}
          ListEmptyComponent={<Text style={styles.empty}>{t("sessions.empty")}</Text>}
          renderItem={renderSession}
          ItemSeparatorComponent={() => <View style={styles.separator} />}
        />

        <View style={styles.footer}>
          <Text style={styles.version}>{t("common.version", { version: VERSION })}</Text>
          <Button
            compact
            icon={<LogOut color={colors.danger} size={16} />}
            label={t("sessions.unpair")}
            onPress={confirmUnpair}
            variant="danger"
          />
        </View>
      </View>
    </SafeAreaView>
  );
}

const styles = StyleSheet.create({
  safe: { flex: 1, backgroundColor: colors.canvas },
  page: { flex: 1, paddingHorizontal: spacing.lg, paddingTop: spacing.lg },
  topbar: {
    flexDirection: "row",
    justifyContent: "space-between",
    alignItems: "flex-start",
    marginBottom: spacing.xl,
  },
  kicker: { color: colors.inkMuted, fontSize: 13, fontWeight: "600", letterSpacing: 0.5 },
  title: { color: colors.inkStrong, fontSize: 28, fontWeight: "700", marginTop: spacing.xs },
  offline: {
    gap: spacing.sm,
    padding: spacing.lg,
    marginBottom: spacing.lg,
    borderRadius: radius.md,
    backgroundColor: colors.warningSoft,
    borderWidth: 1,
    borderColor: colors.warningLine,
  },
  offlineTitle: { color: colors.warning, fontSize: 15, fontWeight: "700" },
  offlineText: { color: colors.inkSoft, fontSize: 14, lineHeight: 20 },
  listHeader: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    marginTop: spacing.xl,
    marginBottom: spacing.sm,
  },
  listTitle: { color: colors.ink, fontSize: 15, fontWeight: "700" },
  list: {
    borderRadius: radius.md,
    borderWidth: 1,
    borderColor: colors.line,
    backgroundColor: colors.surface,
    overflow: "hidden",
  },
  emptyList: { flexGrow: 1, justifyContent: "center" },
  empty: { color: colors.inkMuted, textAlign: "center", fontSize: 15 },
  session: {
    minHeight: 72,
    flexDirection: "row",
    alignItems: "center",
    paddingHorizontal: spacing.lg,
    backgroundColor: colors.surface,
  },
  sessionCopy: { flex: 1, marginRight: spacing.md },
  sessionTitle: { color: colors.ink, fontSize: 16, fontWeight: "600" },
  sessionId: { color: colors.inkMuted, fontSize: 11, marginTop: 5 },
  separator: { height: 1, backgroundColor: colors.lineSoft },
  pressed: { backgroundColor: colors.surfaceSubtle },
  footer: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    paddingVertical: spacing.lg,
  },
  version: { color: colors.inkMuted, fontSize: 12 },
});
