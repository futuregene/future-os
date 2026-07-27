import {
  ChevronRight,
  Folder,
  Link2,
  LogOut,
  MessageCircle,
  Plus,
  Settings,
  X,
} from "lucide-react-native";
import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  Alert,
  FlatList,
  Modal,
  Pressable,
  ScrollView,
  StyleSheet,
  Text,
  View,
} from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";
import { Button } from "../components/Button";
import { useRemote } from "../remote/RemoteContext";
import type { RemoteSession, RemoteWorkspace } from "../remote/types";
import { colors, radius, spacing } from "../theme/tokens";

type Tab = "workspace" | "chat";

export function SessionsScreen() {
  const { t } = useTranslation();
  const remote = useRemote();
  const [tab, setTab] = useState<Tab>("chat");
  const [newOpen, setNewOpen] = useState(false);
  const [newMode, setNewMode] = useState<Tab>("chat");
  const [workspaceId, setWorkspaceId] = useState("");
  const [settingsOpen, setSettingsOpen] = useState(false);

  const chats = useMemo(
    () => remote.sessions.filter(session => session.mode !== "workspace"),
    [remote.sessions],
  );
  const workspaceSessions = useMemo(
    () => remote.sessions.filter(session => session.mode === "workspace"),
    [remote.sessions],
  );

  const openNew = () => {
    const defaultWorkspace = remote.workspaces[0]?.id ?? "";
    setNewMode(tab);
    setWorkspaceId(tab === "workspace" ? defaultWorkspace : "");
    setNewOpen(true);
  };

  const confirmUnpair = () => {
    Alert.alert(t("sessions.unpair"), t("sessions.unpairConfirm"), [
      { text: t("chat.cancel"), style: "cancel" },
      {
        text: t("sessions.unpair"),
        style: "destructive",
        onPress: () => void remote.unpair(),
      },
    ]);
  };

  const startConversation = () => {
    if (newMode === "workspace" && !workspaceId) return;
    setNewOpen(false);
    void remote.newConversation(newMode, workspaceId);
  };

  const renderSession = (item: RemoteSession, inWorkspace = false) => (
    <Pressable
      accessibilityRole="button"
      key={item.sessionId}
      onPress={() => void remote.selectSession(item.sessionId)}
      style={({ pressed }) => [
        styles.session,
        inWorkspace && styles.workspaceSession,
        pressed && styles.pressed,
      ]}
    >
      <Text numberOfLines={1} style={styles.sessionTitle}>
        {item.title || t("sessions.unnamed")}
      </Text>
      <ChevronRight color={colors.inkMuted} size={18} />
    </Pressable>
  );

  const renderWorkspace = ({ item }: { item: RemoteWorkspace }) => {
    const sessions = workspaceSessions.filter(session => session.workspaceId === item.id);
    return (
      <View style={styles.workspaceGroup}>
        <View style={styles.workspaceHeader}>
          <View style={styles.workspaceIcon}>
            <Folder color={colors.accent} size={19} />
          </View>
          <Text numberOfLines={1} style={styles.workspaceName}>
            {item.name}
          </Text>
        </View>
        {sessions.length > 0 ? (
          sessions.map(session => renderSession(session, true))
        ) : (
          <Text style={styles.emptyInside}>{t("sessions.empty")}</Text>
        )}
      </View>
    );
  };

  const connected = remote.desktopOnline;
  return (
    <SafeAreaView edges={["top", "bottom"]} style={styles.safe}>
      <View style={styles.page}>
        <View style={styles.topbar}>
          <View style={styles.tabs}>
            <Pressable
              accessibilityRole="tab"
              accessibilityState={{ selected: tab === "workspace" }}
              onPress={() => setTab("workspace")}
              style={[styles.tab, tab === "workspace" && styles.tabActive]}
            >
              <Folder color={tab === "workspace" ? colors.ink : colors.inkMuted} size={16} />
              <Text style={[styles.tabText, tab === "workspace" && styles.tabTextActive]}>
                {t("sessions.workspace")}
              </Text>
            </Pressable>
            <Pressable
              accessibilityRole="tab"
              accessibilityState={{ selected: tab === "chat" }}
              onPress={() => setTab("chat")}
              style={[styles.tab, tab === "chat" && styles.tabActive]}
            >
              <MessageCircle color={tab === "chat" ? colors.ink : colors.inkMuted} size={16} />
              <Text style={[styles.tabText, tab === "chat" && styles.tabTextActive]}>
                {t("sessions.conversations")}
              </Text>
            </Pressable>
          </View>
          <View style={styles.topActions}>
            <View
              accessibilityLabel={
                connected ? t("connection.connected") : t("connection.reconnecting")
              }
              style={[styles.linkIndicator, connected ? styles.linkOnline : styles.linkPending]}
            >
              <Link2 color={connected ? colors.success : colors.warning} size={17} />
            </View>
            <Pressable
              accessibilityLabel={t("sessions.settings")}
              accessibilityRole="button"
              onPress={() => setSettingsOpen(true)}
              style={styles.settingsButton}
            >
              <Settings color={colors.inkSoft} size={21} />
            </Pressable>
          </View>
        </View>

        {tab === "workspace" ? (
          <FlatList
            contentContainerStyle={
              remote.workspaces.length === 0 ? styles.emptyList : styles.workspaceList
            }
            data={remote.workspaces}
            keyExtractor={item => item.id}
            ListEmptyComponent={<Text style={styles.empty}>{t("sessions.noWorkspaces")}</Text>}
            renderItem={renderWorkspace}
            scrollIndicatorInsets={{ right: 0 }}
            style={styles.list}
          />
        ) : (
          <FlatList
            contentContainerStyle={chats.length === 0 ? styles.emptyList : styles.chatList}
            data={chats}
            ItemSeparatorComponent={() => <View style={styles.listGap} />}
            keyExtractor={item => item.sessionId}
            ListEmptyComponent={<Text style={styles.empty}>{t("sessions.empty")}</Text>}
            renderItem={({ item }) => renderSession(item)}
            scrollIndicatorInsets={{ right: 0 }}
            style={styles.list}
          />
        )}

        <Pressable
          accessibilityLabel={t("sessions.new")}
          accessibilityRole="button"
          onPress={openNew}
          style={({ pressed }) => [styles.fab, pressed && styles.fabPressed]}
        >
          <Plus color={colors.surface} size={27} />
        </Pressable>

        <Modal
          animationType="fade"
          onRequestClose={() => setNewOpen(false)}
          transparent
          visible={newOpen}
        >
          <View style={styles.overlay}>
            <View style={styles.dialog}>
              <View style={styles.dialogHeader}>
                <Text style={styles.dialogTitle}>{t("sessions.new")}</Text>
                <Pressable accessibilityLabel={t("common.close")} onPress={() => setNewOpen(false)}>
                  <X color={colors.inkMuted} size={20} />
                </Pressable>
              </View>
              <View style={styles.modeOptions}>
                {(["workspace", "chat"] as Tab[]).map(mode => (
                  <Pressable
                    key={mode}
                    onPress={() => {
                      setNewMode(mode);
                      if (mode === "workspace" && !workspaceId)
                        setWorkspaceId(remote.workspaces[0]?.id ?? "");
                    }}
                    style={[styles.modeOption, newMode === mode && styles.modeOptionActive]}
                  >
                    {mode === "workspace" ? (
                      <Folder color={colors.accent} size={18} />
                    ) : (
                      <MessageCircle color={colors.accent} size={18} />
                    )}
                    <Text style={styles.modeOptionText}>
                      {mode === "workspace" ? t("sessions.workspace") : t("sessions.conversations")}
                    </Text>
                  </Pressable>
                ))}
              </View>
              {newMode === "workspace" && (
                <ScrollView bounces={false} contentContainerStyle={styles.workspaceOptions}>
                  {remote.workspaces.map(workspace => (
                    <Pressable
                      key={workspace.id}
                      onPress={() => setWorkspaceId(workspace.id)}
                      style={[
                        styles.workspaceOption,
                        workspaceId === workspace.id && styles.workspaceOptionActive,
                      ]}
                    >
                      <Folder color={colors.accent} size={17} />
                      <Text numberOfLines={1} style={styles.workspaceOptionName}>
                        {workspace.name}
                      </Text>
                    </Pressable>
                  ))}
                  {remote.workspaces.length === 0 && (
                    <Text style={styles.emptyInside}>{t("sessions.noWorkspaces")}</Text>
                  )}
                </ScrollView>
              )}
              <Button
                disabled={newMode === "workspace" && !workspaceId}
                label={t("sessions.new")}
                onPress={startConversation}
              />
            </View>
          </View>
        </Modal>

        <Modal
          animationType="fade"
          onRequestClose={() => setSettingsOpen(false)}
          transparent
          visible={settingsOpen}
        >
          <View style={styles.overlay}>
            <View style={styles.dialog}>
              <View style={styles.dialogHeader}>
                <Text style={styles.dialogTitle}>{t("sessions.settings")}</Text>
                <Pressable
                  accessibilityLabel={t("common.close")}
                  onPress={() => setSettingsOpen(false)}
                >
                  <X color={colors.inkMuted} size={20} />
                </Pressable>
              </View>
              <Text style={styles.settingsPlaceholder}>{t("sessions.settingsPlaceholder")}</Text>
              <Button
                icon={<LogOut color={colors.danger} size={16} />}
                label={t("sessions.unpair")}
                onPress={confirmUnpair}
                variant="danger"
              />
            </View>
          </View>
        </Modal>
      </View>
    </SafeAreaView>
  );
}

const styles = StyleSheet.create({
  safe: { flex: 1, backgroundColor: colors.surface },
  page: { flex: 1, backgroundColor: colors.surface },
  topbar: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    paddingHorizontal: spacing.md,
    paddingTop: spacing.md,
    marginBottom: spacing.md,
  },
  tabs: {
    flexDirection: "row",
    gap: spacing.xs,
    padding: spacing.xs,
    borderRadius: radius.md,
    backgroundColor: colors.surfaceSubtle,
  },
  tab: {
    flexDirection: "row",
    alignItems: "center",
    gap: 6,
    paddingHorizontal: spacing.md,
    paddingVertical: spacing.sm,
    borderRadius: radius.sm,
  },
  tabActive: { backgroundColor: colors.surface },
  tabText: { color: colors.inkMuted, fontSize: 13, fontWeight: "600" },
  tabTextActive: { color: colors.ink, fontWeight: "700" },
  topActions: { flexDirection: "row", alignItems: "center", gap: spacing.xs },
  linkIndicator: {
    width: 36,
    height: 36,
    alignItems: "center",
    justifyContent: "center",
    borderRadius: radius.md,
  },
  linkOnline: { backgroundColor: colors.successSoft },
  linkPending: { backgroundColor: colors.warningSoft },
  settingsButton: {
    width: 40,
    height: 40,
    alignItems: "center",
    justifyContent: "center",
    borderRadius: radius.md,
  },
  list: { flex: 1 },
  chatList: { paddingHorizontal: spacing.md, paddingBottom: 84 },
  listGap: { height: 2 },
  workspaceList: { paddingBottom: 84 },
  workspaceGroup: { marginBottom: spacing.lg },
  workspaceHeader: {
    flexDirection: "row",
    alignItems: "center",
    gap: spacing.sm,
    paddingHorizontal: spacing.md,
    paddingTop: spacing.sm,
    paddingBottom: spacing.xs,
  },
  workspaceIcon: {
    width: 28,
    height: 28,
    alignItems: "center",
    justifyContent: "center",
    borderRadius: radius.sm,
    backgroundColor: colors.accentSoft,
  },
  workspaceName: { flex: 1, color: colors.inkSoft, fontSize: 13, fontWeight: "700" },
  session: {
    minHeight: 52,
    flexDirection: "row",
    alignItems: "center",
    gap: spacing.md,
    paddingHorizontal: spacing.md,
  },
  workspaceSession: { paddingLeft: 48 },
  sessionTitle: { flex: 1, color: colors.ink, fontSize: 17, fontWeight: "400", lineHeight: 20 },
  pressed: { backgroundColor: colors.surfaceSubtle },
  emptyList: {
    flexGrow: 1,
    alignItems: "center",
    justifyContent: "center",
    paddingHorizontal: spacing.md,
  },
  empty: { color: colors.inkMuted, fontSize: 14 },
  emptyInside: {
    paddingHorizontal: spacing.md,
    paddingVertical: spacing.sm,
    color: colors.inkMuted,
    fontSize: 13,
  },
  fab: {
    position: "absolute",
    right: spacing.lg,
    bottom: spacing.lg,
    width: 56,
    height: 56,
    alignItems: "center",
    justifyContent: "center",
    borderRadius: radius.pill,
    backgroundColor: colors.accent,
    shadowColor: colors.inkStrong,
    shadowOpacity: 0.2,
    shadowRadius: 8,
    elevation: 5,
  },
  fabPressed: { opacity: 0.8 },
  overlay: {
    flex: 1,
    alignItems: "center",
    justifyContent: "center",
    padding: spacing.xl,
    backgroundColor: colors.overlay,
  },
  dialog: {
    width: "100%",
    maxWidth: 420,
    gap: spacing.md,
    padding: spacing.lg,
    borderRadius: radius.lg,
    backgroundColor: colors.surface,
  },
  dialogHeader: { flexDirection: "row", alignItems: "center", justifyContent: "space-between" },
  dialogTitle: { color: colors.inkStrong, fontSize: 18, fontWeight: "700" },
  modeOptions: { flexDirection: "row", gap: spacing.sm },
  modeOption: {
    flex: 1,
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "center",
    gap: spacing.sm,
    padding: spacing.md,
    borderWidth: 1,
    borderColor: colors.line,
    borderRadius: radius.md,
  },
  modeOptionActive: { borderColor: colors.accent, backgroundColor: colors.accentSoft },
  modeOptionText: { color: colors.ink, fontSize: 13, fontWeight: "700" },
  workspaceOptions: { maxHeight: 180, gap: spacing.xs },
  workspaceOption: {
    flexDirection: "row",
    alignItems: "center",
    gap: spacing.sm,
    padding: spacing.sm,
    borderWidth: 1,
    borderColor: colors.lineSoft,
    borderRadius: radius.sm,
  },
  workspaceOptionActive: { borderColor: colors.accent, backgroundColor: colors.accentSoft },
  workspaceOptionName: { flex: 1, color: colors.ink, fontSize: 14, fontWeight: "600" },
  settingsPlaceholder: { color: colors.inkSoft, fontSize: 14, lineHeight: 20 },
});
