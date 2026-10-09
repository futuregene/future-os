import {
  ChevronLeft,
  Folder,
  MessageCircle,
  Plus,
  Pin,
  Pencil,
  Trash2,
  Settings,
  Unplug,
  X,
} from "lucide-react-native";
import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  Modal,
  Platform,
  Pressable,
  StyleSheet,
  Text,
  TextInput,
  View,
} from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";
import { Button } from "../components/Button";
import { ActionMenu } from "../components/ActionMenu";
import { ConnectionBadge } from "../components/ConnectionBadge";
import { ErrorBanner } from "../components/ErrorBanner";
import { DialogSurface } from "../components/DialogSurface";
import { useAppDialog } from "../components/useAppDialog";
import { RenameModal } from "../features/chat/components/RenameModal";
import { useRemoteControls as useRemote } from "../remote/RemoteContext";
import { useDesktopCatalogs } from "../remote/useDesktopCatalogs";
import { mergeSessions, type DesktopFilter } from "../remote/mergeSessions";
import { iconGlyph } from "../remote/peerIcons";
import type { RemoteSession } from "../remote/types";
import { SessionList } from "./SessionList";
import { DesktopPicker } from "./DesktopPicker";
import { DisconnectedScreen } from "./DisconnectedScreen";
import { colors, layout, radius, spacing } from "../theme/tokens";
import { promptUpgrade } from "../update/prompt";
import { checkForUpdate } from "../update/update";
import { SettingsScreen, type SettingsScreenHandle } from "../features/settings/SettingsScreen";

type Tab = "workspace" | "chat";

// Persist the selected tab across screen transitions (SessionsScreen is
// unmounted when entering a chat and remounted when returning).
let lastTab: Tab = "chat";

function deferPresentation(action: () => void): void {
  // Let the native action sheet finish dismissing before presenting another
  // alert. InteractionManager can remain pending during native transitions.
  setTimeout(action, Platform.OS === "ios" ? 350 : 0);
}

export function SessionsScreen({ onManageDesktops, active = true }: {
  onManageDesktops(): void;
  active?: boolean;
}) {
  const Alert = useAppDialog(active);
  const { t, i18n } = useTranslation();
  const remote = useRemote();
  const selectedDesktop =
    remote.desktops.find((desktop) => desktop.pairId === remote.credentials?.pairId) ??
    remote.desktops[0];
  const [tab, setTabState] = useState<Tab>(lastTab);
  /**
   * Which desktops the session list shows. Kept for the life of the screen: a
   * filter that reset itself on every re-render would look like the list had a
   * mind of its own.
   */
  const [desktopFilter, setDesktopFilter] = useState<DesktopFilter>({ kind: "all" });
  const activeDesktopId = remote.credentials?.expectedDesktopId ?? null;
  // Catalogues for the *other* paired desktops, so the merged list can show
  // their sessions without disturbing the active connection.
  const catalogs = useDesktopCatalogs(activeDesktopId, remote.sessions);
  const mergedRows = useMemo(
    () => mergeSessions(catalogs.catalogs, desktopFilter),
    [catalogs.catalogs, desktopFilter],
  );
  /**
   * The merged view replaces the workspace tree only when it has something the
   * tree cannot show — rows from another desktop. A user with one pairing keeps
   * today's list exactly, because the flat view is a different information
   * architecture (a workspace name can exist on two machines at once).
   */
  const showMerged = useMemo(
    () => mergedRows.some(row => row.desktopId !== activeDesktopId),
    [mergedRows, activeDesktopId],
  );
  const peerIcons = useMemo(() => {
    const map = new Map<string, string>();
    for (const desktop of remote.desktops) map.set(desktop.desktopId, iconGlyph(desktop.icon));
    return map;
  }, [remote.desktops]);
  const setTab = (next: Tab) => {
    lastTab = next;
    setTabState(next);
  };
  const [newOpen, setNewOpen] = useState(false);
  const [newMode, setNewMode] = useState<Tab>("chat");
  const [workspaceId, setWorkspaceId] = useState("");
  // The new-conversation dialog has two views: the workspace picker and, from
  // its "new workspace" row, the create form (the phone has no folder picker
  // for the desktop, so the user types the host path).
  const [newWorkspaceOpen, setNewWorkspaceOpen] = useState(false);
  const [newWorkspaceName, setNewWorkspaceName] = useState("");
  const [newWorkspacePath, setNewWorkspacePath] = useState("");
  const [newWorkspaceError, setNewWorkspaceError] = useState<string | null>(null);
  const [creatingWorkspace, setCreatingWorkspace] = useState(false);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const settingsRef = useRef<SettingsScreenHandle>(null);
  const [checkingUpdate, setCheckingUpdate] = useState(false);
  const [menuSession, setMenuSession] = useState<RemoteSession | null>(null);
  const [renameTarget, setRenameTarget] = useState<RemoteSession | null>(null);
  const [renameValue, setRenameValue] = useState("");
  const pendingNewConversationRef = useRef<(() => void) | null>(null);
  const pendingSettingsActionRef = useRef<(() => void) | null>(null);
  const [wasActive, setWasActive] = useState(active);
  if (wasActive !== active) {
    setWasActive(active);
    if (!active) {
      setNewOpen(false);
      setSettingsOpen(false);
      setMenuSession(null);
      setRenameTarget(null);
      setNewWorkspaceOpen(false);
      setNewWorkspaceError(null);
    }
  }

  const afterSettings = (action: () => void) => {
    pendingSettingsActionRef.current = Platform.OS === "ios" ? action : null;
    setSettingsOpen(false);
    if (Platform.OS !== "ios") deferPresentation(action);
  };

  const flushPendingSettingsAction = () => {
    const action = pendingSettingsActionRef.current;
    pendingSettingsActionRef.current = null;
    action?.();
  };

  const openNew = () => {
    // The conversations tab has nothing left to pick — create the chat
    // straight away instead of showing the mode dialog.
    if (tab === "chat") {
      void remote.newConversation("chat");
      return;
    }
    setNewMode("workspace");
    setWorkspaceId(remote.workspaces[0]?.id ?? "");
    setNewWorkspaceOpen(false);
    setNewWorkspaceError(null);
    setNewOpen(true);
  };

  const openNewWorkspaceForm = () => {
    setNewWorkspaceName("");
    setNewWorkspacePath("");
    setNewWorkspaceError(null);
    setNewWorkspaceOpen(true);
  };

  const closeNewWorkspaceForm = () => {
    setNewWorkspaceOpen(false);
    setNewWorkspaceName("");
    setNewWorkspacePath("");
    setNewWorkspaceError(null);
  };

  const submitNewWorkspace = async () => {
    const path = newWorkspacePath.trim();
    if (!path || creatingWorkspace) return;
    setCreatingWorkspace(true);
    setNewWorkspaceError(null);
    try {
      const workspace = await remote.createWorkspace(path, newWorkspaceName);
      // Back to the picker with the new workspace selected; the user confirms
      // with the ordinary "new conversation" button.
      setWorkspaceId(workspace.id);
      closeNewWorkspaceForm();
    } catch {
      setNewWorkspaceError(t("sessions.createWorkspaceFailed"));
    } finally {
      setCreatingWorkspace(false);
    }
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

  const checkUpdate = async () => {
    setCheckingUpdate(true);
    try {
      const status = await checkForUpdate();
      if (status.hasUpdate) {
        promptUpgrade(status, t);
      } else {
        Alert.alert(t("update.upToDate"));
      }
    } catch {
      Alert.alert(t("update.checkFailed"));
    } finally {
      setCheckingUpdate(false);
    }
  };



  const startConversation = () => {
    if (newMode === "workspace" && !workspaceId) return;
    const mode = newMode;
    const selectedWorkspaceId = workspaceId;
    const start = () => void remote.newConversation(mode, selectedWorkspaceId);
    setNewOpen(false);
    if (Platform.OS === "ios") {
      // Do not switch screens while the creation Modal still owns UIKit's
      // presentation controller. The destination screen may immediately need
      // to present another native surface.
      pendingNewConversationRef.current = start;
    } else {
      deferPresentation(start);
    }
  };

  const closeNew = () => {
    pendingNewConversationRef.current = null;
    setNewOpen(false);
    closeNewWorkspaceForm();
  };

  const flushPendingNewConversation = () => {
    if (Platform.OS !== "ios") return;
    const start = pendingNewConversationRef.current;
    pendingNewConversationRef.current = null;
    start?.();
  };

  const renameSession = async (session: RemoteSession, rawName: string) => {
    const name = rawName.trim();
    if (!name) return;
    try {
      await remote.rename(session.sessionId, name);
    } catch {
      Alert.alert(t("common.error"));
    }
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
    await renameSession(session, name);
  };

  const togglePin = (session: RemoteSession) => {
    void remote
      .setSessionPinned(session.sessionId, session.threadId, !session.pinned)
      .catch(() => Alert.alert(t("common.error")));
  };

  const confirmDelete = (session: RemoteSession) => {
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

  const openSessionMenu = (session: RemoteSession) => {
    if (remote.desktopOnline) setMenuSession(session);
  };

  const connection = remote.connectionPresentation;
  const connected = connection.level === "connected";
  const connecting = connection.level === "connecting";
  const [reconnectingFromDisconnected, setReconnectingFromDisconnected] = useState(false);
  const reconnectStartedRef = useRef(false);

  useEffect(() => {
    if (!reconnectingFromDisconnected) return;
    if (
      remote.phase === "connecting" ||
      remote.phase === "reconnecting" ||
      remote.phase === "refreshing"
    ) {
      reconnectStartedRef.current = true;
      return;
    }
    if (connected || (reconnectStartedRef.current && connection.level === "disconnected")) {
      reconnectStartedRef.current = false;
      setReconnectingFromDisconnected(false);
    }
  }, [connected, connection.level, reconnectingFromDisconnected, remote.phase]);

  const reconnect = () => {
    if (connection.level === "disconnected") {
      reconnectStartedRef.current = false;
      setReconnectingFromDisconnected(true);
    }
    void remote.reconnect();
  };
  const standaloneConnecting =
    reconnectingFromDisconnected || (!remote.hasConnectedContent && connecting);
  const showStandaloneStatus = connection.level === "disconnected" || standaloneConnecting;

  const offlineEmpty = (
    <View style={styles.emptyState}>
      <View
        style={[
          styles.emptyIcon,
          { backgroundColor: connecting ? colors.warningSoft : colors.dangerSoft },
        ]}
      >
        <Unplug color={connecting ? colors.warning : colors.danger} size={26} />
      </View>
      <Text style={styles.emptyTitle}>
        {t(connection.level === "disconnected" ? "connection.disconnected" : connection.titleKey)}
      </Text>
      {connection.level === "disconnected" && (
        <Text style={styles.emptyHint}>{t(connection.hintKey ?? "connection.offlineHint")}</Text>
      )}
      {connection.level === "disconnected" && (
        <Button compact label={t("connection.retry")} onPress={() => void remote.reconnect()} />
      )}
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
    <SafeAreaView style={styles.safe}>
      <View style={styles.page}>
        {Alert.dialog}
        <ActionMenu
          title={menuSession?.title || t("sessions.unnamed")}
          visible={menuSession !== null}
          onClose={() => setMenuSession(null)}
          actions={menuSession ? [
            { label: t(menuSession.pinned ? "sessions.unpin" : "sessions.pin"), icon: <Pin size={18} color={colors.inkSoft} />, disabled: !remote.desktopOnline, onPress: () => togglePin(menuSession) },
            { label: t("chat.rename"), icon: <Pencil size={18} color={colors.inkSoft} />, disabled: !remote.desktopOnline, onPress: () => openRename(menuSession) },
            { label: t("sessions.delete"), icon: <Trash2 size={18} color={colors.danger} />, destructive: true, disabled: !remote.desktopOnline, onPress: () => confirmDelete(menuSession) },
          ] : []}
        />
        <View style={styles.deviceBar}>
          <DesktopPicker
            activeDesktopId={remote.credentials?.expectedDesktopId ?? null}
            desktops={remote.desktops}
            filter={desktopFilter}
            onChange={setDesktopFilter}
            onManage={onManageDesktops}
          />
          <View style={styles.topActions}>
            <ConnectionBadge
              active={active}
              presentation={connection}
              disconnectReason={remote.presence?.reason}
              error={remote.error}
              onReconnect={reconnect}
              onUnpair={confirmUnpair}
            />
            <Pressable
              accessibilityLabel={t("sessions.settings")}
              accessibilityRole="button"
              onPress={() => setSettingsOpen(true)}
              style={({ pressed }) => [styles.settingsButton, pressed && styles.pressed]}
            >
              <Settings color={colors.inkSoft} size={21} />
            </Pressable>
          </View>
        </View>
        {remote.error && connection.level === "connected" && (
          <ErrorBanner
            message={remote.error}
            onDismiss={remote.clearError}
          />
        )}

        {showStandaloneStatus ? (
          <DisconnectedScreen
            reconnecting={standaloneConnecting}
            onReconnect={reconnect}
            onUnpair={confirmUnpair}
          />
        ) : <SessionList
          key={tab}
          tab={tab}
          active={active}
          onTabChange={setTab}
          activeDesktopId={remote.credentials?.expectedDesktopId ?? null}
          icons={peerIcons}
          merged={showMerged ? mergedRows : undefined}
          empty={tab === "workspace" ? workspaceEmpty : !connected ? offlineEmpty : createChatEmpty}
          onMenu={openSessionMenu}
        />}

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
          onDismiss={flushPendingNewConversation}
          onRequestClose={closeNew}
          transparent
          visible={newOpen}
        >
          <DialogSurface footer={newWorkspaceOpen ? (
            <Button
              disabled={creatingWorkspace || !newWorkspacePath.trim()}
              label={t("sessions.createWorkspaceAction")}
              onPress={() => void submitNewWorkspace()}
            />
          ) : (
            <Button
              disabled={newMode === "workspace" && !workspaceId}
              label={t("sessions.new")}
              onPress={startConversation}
            />
          )}>
              {newWorkspaceOpen ? (
                <>
                  <View style={styles.dialogHeader}>
                    <Pressable accessibilityRole="button" accessibilityLabel={t("common.back")} onPress={closeNewWorkspaceForm} style={styles.settingsButton}>
                      <ChevronLeft color={colors.inkMuted} size={20} />
                    </Pressable>
                    <Text style={styles.dialogTitle}>{t("sessions.createWorkspace")}</Text>
                    <Pressable accessibilityRole="button" accessibilityLabel={t("common.close")} onPress={closeNew} style={styles.settingsButton}>
                      <X color={colors.inkMuted} size={20} />
                    </Pressable>
                  </View>
                  <View style={styles.workspaceForm}>
                    <Text style={styles.fieldLabel}>{t("sessions.workspaceName")}</Text>
                    <TextInput
                      accessibilityLabel={t("sessions.workspaceName")}
                      autoCapitalize="sentences"
                      editable={!creatingWorkspace}
                      placeholder={t("sessions.workspaceNamePlaceholder")}
                      placeholderTextColor={colors.inkMuted}
                      style={styles.workspaceInput}
                      value={newWorkspaceName}
                      onChangeText={setNewWorkspaceName}
                    />
                    <Text style={styles.fieldLabel}>{t("sessions.workspacePath")}</Text>
                    <TextInput
                      accessibilityLabel={t("sessions.workspacePath")}
                      autoCapitalize="none"
                      autoCorrect={false}
                      editable={!creatingWorkspace}
                      placeholder={t("sessions.workspacePathPlaceholder")}
                      placeholderTextColor={colors.inkMuted}
                      style={styles.workspaceInput}
                      value={newWorkspacePath}
                      onChangeText={setNewWorkspacePath}
                    />
                    <Text style={styles.fieldHint}>{t("sessions.workspacePathHint")}</Text>
                    {newWorkspaceError && (
                      <Text style={styles.fieldError}>{newWorkspaceError}</Text>
                    )}
                  </View>
                </>
              ) : (
                <>
                  <View style={styles.dialogHeader}>
                    <Text style={styles.dialogTitle}>{t("sessions.new")}</Text>
                    <Pressable accessibilityRole="button" accessibilityLabel={t("common.close")} onPress={closeNew} style={styles.settingsButton}>
                      <X color={colors.inkMuted} size={20} />
                    </Pressable>
                  </View>
                  <View style={styles.modeOptions}>
                    {(["workspace", "chat"] as Tab[]).map((mode) => (
                      <Pressable
                        key={mode}
                        accessibilityRole="radio"
                        accessibilityState={{ checked: newMode === mode }}
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
                    <View style={styles.workspaceOptionsContent}>
                      {remote.workspaces.map((workspace) => (
                        <Pressable
                          key={workspace.id}
                          accessibilityRole="radio"
                          accessibilityState={{ checked: workspaceId === workspace.id }}
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
                      {/* The phone can't browse the host, so it registers a
                          directory the user typed — the same store write as the
                          Desktop's own create-workspace dialog. Gated on the
                          capability: an older desktop answers the command with
                          "Unsupported command". */}
                      {remote.capabilities?.has("workspace_create_v1") && (
                        <Pressable
                          accessibilityRole="button"
                          accessibilityLabel={t("sessions.createWorkspace")}
                          onPress={openNewWorkspaceForm}
                          style={styles.workspaceOption}
                        >
                          <Plus color={colors.accent} size={17} />
                          <Text numberOfLines={1} style={styles.workspaceOptionName}>
                            {t("sessions.createWorkspace")}
                          </Text>
                        </Pressable>
                      )}
                      {remote.workspaces.length === 0 && (
                        <Text style={styles.emptyInside}>{t("sessions.noWorkspaces")}</Text>
                      )}
                    </View>
                  )}
                </>
              )}
          </DialogSurface>
        </Modal>

        <Modal
          animationType="slide"
          presentationStyle="fullScreen"
          onRequestClose={() => settingsRef.current?.goBack()}
          onDismiss={flushPendingSettingsAction}
          visible={settingsOpen}
        >
          {settingsOpen ? <SettingsScreen
            ref={settingsRef}
            key={`${remote.credentials?.pairId}:${remote.desktopOnline}`}
            onClose={() => setSettingsOpen(false)}
            onCheckUpdate={() => afterSettings(() => void checkUpdate())}
            checkingUpdate={checkingUpdate}
            onOpenConversation={sessionId => afterSettings(() => void remote.selectSession(sessionId))}
          /> : null}
        </Modal>

        <RenameModal
          renameOpen={renameTarget !== null}
          generationKey={renameTarget?.sessionId}
          onGenerate={renameTarget && remote.desktopOnline
            ? () => remote.generateTitle(renameTarget.sessionId, i18n.language.startsWith("zh") ? "zh" : "en")
            : undefined}
          renameValue={renameValue}
          setRenameValue={setRenameValue}
          submitRename={submitRename}
          onClose={() => setRenameTarget(null)}
          t={t}
        />
      </View>
    </SafeAreaView>
  );
}

const styles = StyleSheet.create({
  deviceBar: {
    flexDirection: "row",
    alignItems: "center",
    gap: spacing.md,
    paddingHorizontal: layout.gutter,
    paddingVertical: spacing.sm,
  },
  desktopSelector: {
    flex: 1,
    minWidth: 0,
    minHeight: layout.touchTarget,
    flexDirection: "row",
    alignItems: "center",
    gap: spacing.sm,
    paddingHorizontal: spacing.sm,
    borderRadius: radius.lg,
  },
  desktopIcon: {
    width: 36,
    height: 36,
    alignItems: "center",
    justifyContent: "center",
    borderRadius: radius.lg,
    backgroundColor: colors.accentSoft,
  },
  desktopIdentity: { flex: 1, minWidth: 0, color: colors.ink, fontSize: 15, fontWeight: "600" },
  safe: { flex: 1, backgroundColor: colors.surface },
  page: { flex: 1, width: "100%", maxWidth: layout.contentMaxWidth, alignSelf: "center", backgroundColor: colors.surface },
  topActions: { flexDirection: "row", alignItems: "center", gap: spacing.xs },
  settingsButton: {
    width: 44,
    height: 44,
    alignItems: "center",
    justifyContent: "center",
    borderRadius: radius.md,
  },
  pressed: { backgroundColor: colors.surfaceSubtle },
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
    marginBottom: spacing.md,
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
  dialogHeader: { flexDirection: "row", alignItems: "center", justifyContent: "space-between" },
  dialogTitle: { flexShrink: 1, color: colors.inkStrong, fontSize: 20, fontWeight: "700" },
  modeOptions: { flexDirection: "row", gap: spacing.sm },
  modeOption: {
    flex: 1,
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "center",
    gap: spacing.sm,
    minHeight: 48,
    padding: spacing.sm,
    borderWidth: 1,
    borderColor: colors.line,
    borderRadius: radius.md,
  },
  modeOptionActive: { borderColor: colors.accent, backgroundColor: colors.accentSoft },
  modeOptionText: { flexShrink: 1, color: colors.ink, fontSize: 14, fontWeight: "600" },
  workspaceOptionsContent: { gap: spacing.xs },
  workspaceOption: {
    minHeight: layout.touchTarget,
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
  workspaceForm: { gap: spacing.xs },
  fieldLabel: { color: colors.ink, fontSize: 14, fontWeight: "600" },
  workspaceInput: {
    minHeight: layout.touchTarget,
    paddingHorizontal: spacing.md,
    paddingVertical: spacing.sm,
    borderWidth: 1,
    borderColor: colors.line,
    borderRadius: radius.md,
    color: colors.ink,
    fontSize: 15,
  },
  fieldHint: { color: colors.inkMuted, fontSize: 13, lineHeight: 18 },
  fieldError: { color: colors.danger, fontSize: 13, lineHeight: 18 },
  settingsLabel: {
    color: colors.inkMuted,
    fontSize: 12,
    fontWeight: "700",
    textTransform: "uppercase",
    letterSpacing: 0.5,
  },
  updateRow: {
    flexDirection: "row",
    flexWrap: "wrap",
    alignItems: "center",
    justifyContent: "space-between",
    gap: spacing.md,
  },
  updateVersion: { color: colors.inkMuted, fontSize: 13 },
  approvalOptions: { flexDirection: "row", gap: spacing.sm },
  tierTriggerDisabled: { backgroundColor: colors.surfaceSubtle, opacity: 0.55 },
});
