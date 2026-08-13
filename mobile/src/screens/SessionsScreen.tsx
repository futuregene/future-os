import {
  CircleAlert,
  Folder,
  LogOut,
  MessageCircle,
  Pencil,
  Pin,
  PinOff,
  Plus,
  Settings,
  Trash2,
  Unplug,
  X,
} from "lucide-react-native";
import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  ActivityIndicator,
  Alert,
  FlatList,
  Modal,
  Pressable,
  ScrollView,
  StyleSheet,
  Text,
  TextInput,
  TouchableWithoutFeedback,
  View,
} from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";
import { Button } from "../components/Button";
import { ConnectionBadge } from "../components/ConnectionBadge";
import { ErrorBanner } from "../components/ErrorBanner";
import { useRemote } from "../remote/RemoteContext";
import { effectiveRunStatus } from "../remote/sessionStatus";
import type { RemoteSession, RemoteWorkspace } from "../remote/types";
import { colors, radius, spacing } from "../theme/tokens";

type Tab = "workspace" | "chat";

// Persist the selected tab across screen transitions (SessionsScreen is
// unmounted when entering a chat and remounted when returning).
let lastTab: Tab = "chat";

function SessionStatusIndicator({
  status,
  streaming,
  unread,
}: {
  status?: string;
  streaming?: boolean;
  unread?: boolean;
}) {
  // Desktop parity (ThreadListItem): a local running/queued status wins, but a
  // session the agent reports as streaming with no local run row (a prompt
  // started by the TUI/CLI/another machine) still reads as running.
  const effective = effectiveRunStatus(status, streaming);
  if (effective === "running" || effective === "queued") {
    return (
      <View style={styles.indicator}>
        <ActivityIndicator color={colors.accent} size={14} />
      </View>
    );
  }
  // A session waiting for approval is distinguishable from a run in flight —
  // the desktop sidebar flags it with a warning glyph so "running" and "waiting
  // on you" don't read the same.
  if (effective === "waiting_approval") {
    return (
      <View style={styles.indicator}>
        <CircleAlert color={colors.warning} size={16} />
      </View>
    );
  }
  if (unread && effective === "completed") {
    return (
      <View style={styles.indicator}>
        <View style={[styles.statusDot, styles.statusCompleted]} />
      </View>
    );
  }
  if (unread && effective === "failed") {
    return (
      <View style={styles.indicator}>
        <View style={[styles.statusDot, styles.statusFailed]} />
      </View>
    );
  }
  return <View style={styles.indicator} />;
}

// Scroll offsets survive the list's unmount when the user dives into a
// conversation and back (and tab switches, which also remount the lists).
const listScrollOffsets: Record<Tab, number> = { chat: 0, workspace: 0 };

export function SessionsScreen() {
  const { t } = useTranslation();
  const remote = useRemote();
  const [tab, setTabState] = useState<Tab>(lastTab);
  const setTab = (next: Tab) => {
    lastTab = next;
    setTabState(next);
  };
  const [newOpen, setNewOpen] = useState(false);
  const [newMode, setNewMode] = useState<Tab>("chat");
  const [workspaceId, setWorkspaceId] = useState("");
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [menuSession, setMenuSession] = useState<RemoteSession | null>(null);
  const [renameTarget, setRenameTarget] = useState<RemoteSession | null>(null);
  const [renameValue, setRenameValue] = useState("");

  const chats = useMemo(
    () => remote.sessions.filter(session => session.mode !== "workspace"),
    [remote.sessions],
  );
  const workspaceSessions = useMemo(
    () => remote.sessions.filter(session => session.mode === "workspace"),
    [remote.sessions],
  );

  const openNew = () => {
    // The conversations tab has nothing left to pick — create the chat
    // straight away instead of showing the mode dialog.
    if (tab === "chat") {
      void remote.newConversation("chat");
      return;
    }
    setNewMode("workspace");
    setWorkspaceId(remote.workspaces[0]?.id ?? "");
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

  const openRename = (session: RemoteSession) => {
    setRenameTarget(session);
    setRenameValue(session.title || "");
  };

  const submitRename = async () => {
    const session = renameTarget;
    const name = renameValue.trim();
    if (!session || !name) return;
    setRenameTarget(null);
    try {
      await remote.rename(session.sessionId, name);
    } catch {
      Alert.alert(t("common.error"));
    }
  };

  const togglePin = (session: RemoteSession) => {
    setMenuSession(null);
    void remote
      .setSessionPinned(session.sessionId, session.threadId, !session.pinned)
      .catch(() => Alert.alert(t("common.error")));
  };

  const confirmDelete = (session: RemoteSession) => {
    setMenuSession(null);
    Alert.alert(
      t("sessions.delete"),
      t("sessions.deleteConfirm", {
        title: session.title || t("sessions.unnamed"),
      }),
      [
        { text: t("chat.cancel"), style: "cancel" },
        {
          text: t("sessions.delete"),
          style: "destructive",
          onPress: () => {
            void remote
              .deleteSession(session.sessionId, session.threadId)
              .catch(() => Alert.alert(t("common.error")));
          },
        },
      ],
    );
  };

  const renderSession = (item: RemoteSession, inWorkspace = false) => (
    <Pressable
      accessibilityRole="button"
      key={item.sessionId}
      onLongPress={() => {
        // Session management goes through the desktop bridge — with the link
        // down every action would just fail, so don't offer the menu.
        if (remote.desktopOnline) setMenuSession(item);
      }}
      onPress={() => {
        // Opening a conversation needs the desktop for state/history — a tap
        // while offline would just land on an error, so keep the list inert.
        if (remote.desktopOnline) void remote.selectSession(item.sessionId);
      }}
      style={({ pressed }) => [
        styles.session,
        inWorkspace && styles.workspaceSession,
        pressed && styles.pressed,
      ]}
    >
      <Text numberOfLines={1} style={styles.sessionTitle}>
        {item.title || t("sessions.unnamed")}
      </Text>
      <SessionStatusIndicator
        status={item.status}
        streaming={item.streaming}
        unread={remote.unreadSessions.has(item.sessionId)}
      />
      {item.pinned && (
        <Pin accessibilityLabel={t("sessions.pin")} color={colors.accent} size={16} />
      )}
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

  const offlineEmpty = (
    <View style={styles.emptyState}>
      <View style={[styles.emptyIcon, styles.emptyIconOffline]}>
        <Unplug color={colors.danger} size={26} />
      </View>
      <Text style={styles.emptyTitle}>{t("connection.offline")}</Text>
      <Text style={styles.emptyHint}>{t("connection.offlineHint")}</Text>
    </View>
  );

  const createChatEmpty = (
    <View style={styles.emptyState}>
      <View style={[styles.emptyIcon, styles.emptyIconIdle]}>
        <MessageCircle color={colors.accent} size={26} />
      </View>
      <Text style={styles.emptyTitle}>{t("sessions.emptyConnectedTitle")}</Text>
      <Text style={styles.emptyHint}>{t("sessions.emptyConnectedHint")}</Text>
      <Button
        compact
        icon={<Plus color={colors.surface} size={16} />}
        label={t("sessions.new")}
        onPress={openNew}
      />
    </View>
  );

  const workspaceEmpty = !connected ? (
    offlineEmpty
  ) : (
    <Text style={styles.empty}>{t("sessions.noWorkspaces")}</Text>
  );

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
            <ConnectionBadge phase={remote.phase} desktopOnline={remote.desktopOnline} />
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

        {remote.error && <ErrorBanner message={remote.error} onDismiss={remote.clearError} />}

        {tab === "workspace" ? (
          <FlatList
            contentContainerStyle={
              remote.workspaces.length === 0 ? styles.emptyList : styles.workspaceList
            }
            contentOffset={{ x: 0, y: listScrollOffsets.workspace }}
            data={remote.workspaces}
            keyExtractor={item => item.id}
            ListEmptyComponent={workspaceEmpty}
            onScroll={event => {
              listScrollOffsets.workspace = event.nativeEvent.contentOffset.y;
            }}
            renderItem={renderWorkspace}
            scrollEventThrottle={16}
            scrollIndicatorInsets={{ right: 0 }}
            style={styles.list}
          />
        ) : (
          <FlatList
            contentContainerStyle={chats.length === 0 ? styles.emptyList : styles.chatList}
            contentOffset={{ x: 0, y: listScrollOffsets.chat }}
            data={chats}
            ItemSeparatorComponent={() => <View style={styles.listGap} />}
            keyExtractor={item => item.sessionId}
            ListEmptyComponent={!connected ? offlineEmpty : createChatEmpty}
            onScroll={event => {
              listScrollOffsets.chat = event.nativeEvent.contentOffset.y;
            }}
            renderItem={({ item }) => renderSession(item)}
            scrollEventThrottle={16}
            scrollIndicatorInsets={{ right: 0 }}
            style={styles.list}
          />
        )}

        {connected && (
          <Pressable
            accessibilityLabel={t("sessions.new")}
            accessibilityRole="button"
            onPress={openNew}
            style={({ pressed }) => [styles.fab, pressed && styles.fabPressed]}
          >
            <Plus color={colors.surface} size={27} />
          </Pressable>
        )}

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
              <Text style={styles.settingsLabel}>{t("approvalTier.title")}</Text>
              {(["manual", "sandbox", "off"] as const)
                .filter(tier => tier !== "sandbox" || remote.sandboxAvailable)
                .map(tier => {
                  const active = remote.approvalTier === tier;
                  return (
                    <Pressable
                      accessibilityRole="button"
                      key={tier}
                      onPress={() =>
                        void remote
                          .setApprovalTier(tier)
                          .catch(() => Alert.alert(t("common.error")))
                      }
                      style={[styles.tierOption, active && styles.tierOptionActive]}
                    >
                      <Text style={[styles.tierOptionText, active && styles.tierOptionTextActive]}>
                        {t(`approvalTier.${tier}`)}
                      </Text>
                    </Pressable>
                  );
                })}
              <Button
                icon={<LogOut color={colors.danger} size={16} />}
                label={t("sessions.unpair")}
                onPress={confirmUnpair}
                variant="danger"
              />
            </View>
          </View>
        </Modal>

        <Modal
          animationType="fade"
          onRequestClose={() => setMenuSession(null)}
          transparent
          visible={menuSession !== null}
        >
          <TouchableWithoutFeedback onPress={() => setMenuSession(null)}>
            <View style={styles.menuOverlay}>
              <TouchableWithoutFeedback>
                <View style={styles.menu}>
                  <Text numberOfLines={1} style={styles.menuTitle}>
                    {menuSession?.title || t("sessions.unnamed")}
                  </Text>
                  <Pressable
                    accessibilityRole="button"
                    onPress={() => {
                      if (menuSession) togglePin(menuSession);
                    }}
                    style={({ pressed }) => [styles.menuOption, pressed && styles.pressed]}
                  >
                    {menuSession?.pinned ? (
                      <PinOff color={colors.ink} size={18} />
                    ) : (
                      <Pin color={colors.ink} size={18} />
                    )}
                    <Text style={styles.menuOptionText}>
                      {menuSession?.pinned ? t("sessions.unpin") : t("sessions.pin")}
                    </Text>
                  </Pressable>
                  <Pressable
                    accessibilityRole="button"
                    onPress={() => {
                      if (menuSession) {
                        openRename(menuSession);
                        setMenuSession(null);
                      }
                    }}
                    style={({ pressed }) => [styles.menuOption, pressed && styles.pressed]}
                  >
                    <Pencil color={colors.ink} size={18} />
                    <Text style={styles.menuOptionText}>{t("chat.rename")}</Text>
                  </Pressable>
                  <Pressable
                    accessibilityRole="button"
                    onPress={() => {
                      if (menuSession) confirmDelete(menuSession);
                    }}
                    style={({ pressed }) => [styles.menuOption, pressed && styles.pressed]}
                  >
                    <Trash2 color={colors.danger} size={18} />
                    <Text style={[styles.menuOptionText, styles.menuOptionDanger]}>
                      {t("sessions.delete")}
                    </Text>
                  </Pressable>
                  <Pressable
                    accessibilityRole="button"
                    onPress={() => setMenuSession(null)}
                    style={({ pressed }) => [
                      styles.menuOption,
                      styles.menuOptionCancel,
                      pressed && styles.pressed,
                    ]}
                  >
                    <Text style={styles.menuOptionCancelText}>{t("chat.cancel")}</Text>
                  </Pressable>
                </View>
              </TouchableWithoutFeedback>
            </View>
          </TouchableWithoutFeedback>
        </Modal>

        <Modal
          animationType="fade"
          onRequestClose={() => setRenameTarget(null)}
          transparent
          visible={renameTarget !== null}
        >
          <View style={styles.overlay}>
            <View style={styles.dialog}>
              <Text style={styles.dialogTitle}>{t("chat.renameTitle")}</Text>
              <TextInput
                autoFocus
                onChangeText={setRenameValue}
                onSubmitEditing={() => void submitRename()}
                placeholder={t("sessions.unnamed")}
                placeholderTextColor={colors.inkMuted}
                returnKeyType="done"
                style={styles.nameInput}
                value={renameValue}
              />
              <View style={styles.dialogActions}>
                <View style={styles.dialogAction}>
                  <Button
                    compact
                    label={t("chat.cancel")}
                    onPress={() => setRenameTarget(null)}
                    variant="secondary"
                  />
                </View>
                <View style={styles.dialogAction}>
                  <Button
                    compact
                    disabled={!renameValue.trim()}
                    label={t("chat.save")}
                    onPress={() => void submitRename()}
                  />
                </View>
              </View>
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
  workspaceName: { flex: 1, color: colors.inkSoft, fontSize: 16, fontWeight: "700" },
  session: {
    minHeight: 52,
    flexDirection: "row",
    alignItems: "center",
    gap: spacing.md,
    paddingHorizontal: spacing.md,
  },
  workspaceSession: { paddingLeft: 48 },
  sessionTitle: { flex: 1, color: colors.ink, fontSize: 17, fontWeight: "400", lineHeight: 20 },
  indicator: { width: 20, height: 20, alignItems: "center", justifyContent: "center" },
  statusDot: { width: 8, height: 8, borderRadius: radius.pill },
  statusCompleted: { backgroundColor: colors.success },
  statusFailed: { backgroundColor: colors.danger },
  pressed: { backgroundColor: colors.surfaceSubtle },
  emptyList: {
    flexGrow: 1,
    alignItems: "center",
    justifyContent: "center",
    paddingHorizontal: spacing.md,
  },
  empty: { color: colors.inkMuted, fontSize: 14 },
  emptyState: {
    width: "100%",
    alignItems: "center",
    gap: spacing.sm,
    paddingHorizontal: spacing.xl,
  },
  emptyIcon: {
    width: 56,
    height: 56,
    alignItems: "center",
    justifyContent: "center",
    borderRadius: radius.lg,
  },
  emptyIconOffline: { backgroundColor: colors.dangerSoft },
  emptyIconIdle: { backgroundColor: colors.accentSoft },
  emptyTitle: {
    alignSelf: "stretch",
    textAlign: "center",
    color: colors.ink,
    fontSize: 16,
    fontWeight: "700",
  },
  emptyHint: {
    alignSelf: "stretch",
    textAlign: "center",
    color: colors.inkMuted,
    fontSize: 13,
    lineHeight: 19,
  },
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
  settingsLabel: {
    color: colors.inkMuted,
    fontSize: 12,
    fontWeight: "700",
    textTransform: "uppercase",
    letterSpacing: 0.5,
  },
  tierOption: {
    padding: spacing.md,
    borderWidth: 1,
    borderColor: colors.line,
    borderRadius: radius.md,
  },
  tierOptionActive: { borderColor: colors.accent, backgroundColor: colors.accentSoft },
  tierOptionText: { color: colors.ink, fontSize: 14, fontWeight: "600" },
  tierOptionTextActive: { color: colors.accent },
  menuOverlay: {
    flex: 1,
    justifyContent: "flex-end",
    backgroundColor: colors.overlay,
  },
  menu: {
    gap: spacing.xs,
    paddingHorizontal: spacing.md,
    paddingTop: spacing.md,
    paddingBottom: spacing.xl,
    backgroundColor: colors.surface,
    borderTopLeftRadius: radius.lg,
    borderTopRightRadius: radius.lg,
  },
  menuTitle: {
    color: colors.inkMuted,
    fontSize: 16,
    fontWeight: "600",
    paddingHorizontal: spacing.md,
    paddingTop: spacing.xs,
    paddingBottom: spacing.sm,
  },
  menuOption: {
    flexDirection: "row",
    alignItems: "center",
    gap: spacing.md,
    padding: spacing.md,
    borderRadius: radius.md,
  },
  menuOptionText: { color: colors.ink, fontSize: 16, fontWeight: "500" },
  menuOptionDanger: { color: colors.danger, fontWeight: "600" },
  menuOptionCancel: { justifyContent: "center" },
  menuOptionCancelText: { color: colors.inkSoft, fontSize: 16, fontWeight: "600" },
  nameInput: {
    borderWidth: 1,
    borderColor: colors.line,
    borderRadius: radius.md,
    paddingHorizontal: spacing.md,
    paddingVertical: spacing.sm,
    color: colors.ink,
    fontSize: 15,
  },
  dialogActions: { flexDirection: "row", gap: spacing.md },
  dialogAction: { flex: 1 },
});
