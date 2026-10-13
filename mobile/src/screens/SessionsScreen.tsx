import {
  Check,
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
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  ActivityIndicator,
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
import { activeMachineStatus, machineStatusKey, type MachineSummary } from "../remote/desktopStatus";
import { useDesktopCatalogs } from "../remote/useDesktopCatalogs";
import { mergeSessions, type DesktopFilter } from "../remote/mergeSessions";
import type { DesktopCommands } from "../remote/desktopCommands";
import { iconGlyph } from "../remote/peerIcons";
import type { RemoteSession, RemoteWorkspace } from "../remote/types";
import { SessionList } from "./SessionList";
import { DesktopPicker } from "./DesktopPicker";
import { DisconnectedScreen } from "./DisconnectedScreen";
import { colors, layout, radius, spacing } from "../theme/tokens";
import { promptUpgrade } from "../update/prompt";
import { checkForUpdate } from "../update/update";
import { SettingsScreen, type SettingsScreenHandle } from "../features/settings/SettingsScreen";

type Tab = "workspace" | "chat";

/** A conversation row, with the machine that owns it. */
interface SessionRow {
  session: RemoteSession;
  /** The desktop the row came from; null for the ordinary single-machine list. */
  desktopId: string | null;
}

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
  /**
   * The commands a row's own machine answers.
   *
   * The row belongs to the machine it came from, so its rename, pin, delete and
   * workspace actions go there: through this screen's own connection for the
   * desktop the phone is talking to, and down that other machine's own
   * connection for a row merged in from it. Both speak the same commands (a
   * desktop answers a paired client with the same ones it answers its own
   * window with), so everything that acts on a row holds one of these — and a
   * machine that cannot take a command answers null, which is what keeps an
   * action from being offered where it could only fail.
   */
  const activeCommands = useMemo<DesktopCommands>(() => ({
    capabilities: remote.capabilities,
    renameSession: remote.rename,
    generateTitle: remote.generateTitle,
    setSessionPinned: remote.setSessionPinned,
    deleteSession: remote.deleteSession,
    renameWorkspace: remote.renameWorkspace,
    setWorkspacePinned: remote.setWorkspacePinned,
    deleteWorkspace: remote.deleteWorkspace,
  }), [
    remote.capabilities,
    remote.deleteSession,
    remote.deleteWorkspace,
    remote.generateTitle,
    remote.rename,
    remote.renameWorkspace,
    remote.setSessionPinned,
    remote.setWorkspacePinned,
  ]);
  /** How every other paired machine stands, by desktop id. */
  const peerReady = useMemo(
    () => new Set(
      catalogs.machines
        .filter(machine => machine.status === "connected")
        .map(machine => machine.desktopId),
    ),
    [catalogs.machines],
  );
  const commandsFor = useCallback(
    (desktopId: string | null): DesktopCommands | null => {
      // A row with no source is the active desktop's (the ordinary list has one
      // machine), and that machine answers only while its connection says so —
      // the same judgement the badge makes, since `desktopOnline` is the
      // presence heartbeat it renders from.
      if (!desktopId || desktopId === activeDesktopId)
        return remote.desktopOnline ? activeCommands : null;
      // A peer answers while its own status says `connected` — the badge's green,
      // the one state in which a host takes work at all: while its agent restarts
      // it refuses, and after its heartbeat stops there may be nothing there. Its
      // connection hands out commands only from that state, so the two cannot
      // disagree about a row.
      return peerReady.has(desktopId) ? catalogs.commandsFor(desktopId) : null;
    },
    [activeCommands, activeDesktopId, catalogs, peerReady, remote.desktopOnline],
  );
  /**
   * Whether a row's own machine can answer its commands right now.
   *
   * A peer counts only while it is `connected` — the badge's own green, the one
   * state in which a host answers at all (transport ready, heartbeat fresh,
   * agent running) — which is exactly when its connection hands out commands.
   * The active machine is read from the connection this screen renders from.
   */
  const canCommand = (desktopId: string | null): boolean => commandsFor(desktopId) !== null;
  const setTab = (next: Tab) => {
    lastTab = next;
    setTabState(next);
  };
  const [newOpen, setNewOpen] = useState(false);
  const [newMode, setNewMode] = useState<Tab>("chat");
  const [workspaceId, setWorkspaceId] = useState("");
  /**
   * Which view of the creation dialog is up: the machine picker, or what to
   * create on the machine it named.
   */
  const [newStep, setNewStep] = useState<"device" | "target">("target");
  /**
   * Whether *this* dialog opened on the machine picker.
   *
   * Decided when it opens rather than read from the live machine list, so a
   * machine dropping out mid-dialog cannot make the step the user is on vanish
   * under them.
   */
  const [newDeviceStep, setNewDeviceStep] = useState(false);
  /** The machine the picker named; null means the one the phone is on. */
  const [newDeviceId, setNewDeviceId] = useState<string | null>(null);
  /**
   * The picked machine's workspaces, read from that machine while the phone is
   * still on another one. Null means "whatever the connected machine has".
   */
  const [deviceWorkspaces, setDeviceWorkspaces] = useState<RemoteWorkspace[] | null>(null);
  /** The machine being read or connected; the picker's spinner and lock. */
  const [busyDevice, setBusyDevice] = useState<string | null>(null);
  /**
   * The dialog's generation.
   *
   * Moving the phone to another machine is asynchronous and the user can close
   * the dialog while it runs, so the step that started it acts only if the
   * dialog it belongs to is still the one on screen: an abandoned choice must
   * not create a conversation.
   */
  const newDialogEpochRef = useRef(0);
  // The new-conversation dialog has three views: the machine picker, the
  // workspace picker, and the create-workspace form behind its "new workspace"
  // row (the phone has no folder picker for the desktop, so the user types the
  // host path).
  const [newWorkspaceOpen, setNewWorkspaceOpen] = useState(false);
  const [newWorkspaceName, setNewWorkspaceName] = useState("");
  const [newWorkspacePath, setNewWorkspacePath] = useState("");
  const [newWorkspaceError, setNewWorkspaceError] = useState<string | null>(null);
  const [creatingWorkspace, setCreatingWorkspace] = useState(false);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const settingsRef = useRef<SettingsScreenHandle>(null);
  const [checkingUpdate, setCheckingUpdate] = useState(false);
  const [menuSession, setMenuSession] = useState<SessionRow | null>(null);
  // Which conversation a dialog is editing, and which machine owns it: every
  // action below is addressed to that machine, not to the phone.
  const [renameTarget, setRenameTarget] = useState<SessionRow | null>(null);
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
      setNewDeviceStep(false);
      setNewDeviceId(null);
      setDeviceWorkspaces(null);
      setBusyDevice(null);
    }
  }

  useEffect(() => {
    // The dialog goes with the screen, so a machine switch that outlives it
    // must not open a conversation on whatever replaces this screen — the same
    // guard `closeNew` puts on the user closing the dialog themselves.
    if (!active) newDialogEpochRef.current += 1;
  }, [active]);

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

  /**
   * Move the phone onto `desktopId`, so the conversation about to be created
   * belongs to that machine, and re-read what building it needs.
   *
   * Resolves false — after saying why — when the machine could not be reached,
   * in which case nothing has been created and the dialog is still worth
   * showing.
   */
  const adoptDevice = async (desktopId: string): Promise<boolean> => {
    setBusyDevice(desktopId);
    try {
      await remote.switchDesktop(desktopId);
      // The catalogue of a freshly connected machine starts empty — the switch
      // drops the previous machine's — so read what the draft is built from
      // before opening it: its models are what the new conversation defaults
      // to, and its workspaces are what the picker offered.
      await Promise.all([remote.refreshWorkspaces(), remote.refreshModels()]);
      return true;
    } catch {
      setBusyDevice(null);
      Alert.alert(t("desktops.switchFailed"));
      return false;
    }
  };

  /**
   * Take the machine the user picked.
   *
   * A machine the phone is not on is read down *its own* connection before the
   * next step, because that machine's workspaces are what the picker must
   * offer — the connected machine's list names workspaces that machine has
   * never heard of. The switch itself waits for the creation to be confirmed:
   * browsing the picker must not move the phone.
   */
  const chooseDevice = async (desktopId: string) => {
    if (busyDevice) return;
    const epoch = newDialogEpochRef.current;
    setNewDeviceId(desktopId);
    if (tab === "chat") {
      // Nothing else to pick for a chat, so the machine is the whole choice.
      if (desktopId !== activeDesktopId && !(await adoptDevice(desktopId))) return;
      if (newDialogEpochRef.current !== epoch) return;
      startConversationOn("chat", "");
      return;
    }
    if (desktopId === activeDesktopId) {
      selectDeviceWorkspaces(null);
      setNewStep("target");
      return;
    }
    setBusyDevice(desktopId);
    let workspaces: RemoteWorkspace[];
    try {
      workspaces = await catalogs.readWorkspaces(desktopId);
    } catch {
      if (newDialogEpochRef.current !== epoch) return;
      setNewDeviceId(null);
      Alert.alert(t("common.error"));
      return;
    } finally {
      setBusyDevice(null);
    }
    if (newDialogEpochRef.current !== epoch) return;
    selectDeviceWorkspaces(workspaces);
    setNewStep("target");
  };

  /**
   * Point the dialog at one machine's workspaces, with its first one selected.
   *
   * A workspace id belongs to the machine it came from, so the selection is
   * replaced rather than carried over: a conversation created in a workspace
   * the machine has never heard of is worse than asking again.
   */
  const selectDeviceWorkspaces = (workspaces: RemoteWorkspace[] | null) => {
    setDeviceWorkspaces(workspaces);
    setWorkspaceId((workspaces ?? remote.workspaces)[0]?.id ?? "");
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



  /**
   * Open the new conversation on the machine the dialog named.
   *
   * The draft is local, but the conversation is not: it becomes a session on
   * whichever machine the phone is connected to when the first message is sent.
   * So a draft the user asked to create on another machine is opened *there*,
   * which is the switch a merged row already makes before it opens a
   * conversation on the machine that owns it.
   */
  const startConversationOn = (mode: "chat" | "workspace", selectedWorkspaceId: string) => {
    const start = () => void remote.newConversation(mode, selectedWorkspaceId);
    setNewOpen(false);
    setBusyDevice(null);
    if (Platform.OS === "ios") {
      // Do not switch screens while the creation Modal still owns UIKit's
      // presentation controller. The destination screen may immediately need
      // to present another native surface.
      pendingNewConversationRef.current = start;
    } else {
      deferPresentation(start);
    }
  };

  const startConversation = async () => {
    if (newMode === "workspace" && !workspaceId) return;
    const epoch = newDialogEpochRef.current;
    const deviceId = newDeviceId;
    const mode = newMode;
    const selectedWorkspaceId = workspaceId;
    // The switch happens at the moment the user commits to creating something
    // on that machine — browsing the picker must not move the phone — and a
    // machine that cannot be reached creates nothing rather than opening a
    // draft that would be sent to the machine the phone is still on.
    if (deviceId && deviceId !== activeDesktopId) {
      if (!(await adoptDevice(deviceId))) return;
      if (newDialogEpochRef.current !== epoch) return;
    }
    startConversationOn(mode, selectedWorkspaceId);
  };

  const closeNew = () => {
    pendingNewConversationRef.current = null;
    newDialogEpochRef.current += 1;
    setNewOpen(false);
    setBusyDevice(null);
    closeNewWorkspaceForm();
  };

  const flushPendingNewConversation = () => {
    if (Platform.OS !== "ios") return;
    const start = pendingNewConversationRef.current;
    pendingNewConversationRef.current = null;
    start?.();
  };

  /**
   * Run one row's command on the machine that owns it, reporting a command that
   * cannot run at all.
   *
   * A machine can go away between the sheet opening and an item being tapped —
   * the trigger disables itself when that happens, but the sheet is already up —
   * and an action that silently does nothing is worse than an error the reader
   * can act on.
   */
  const runCommand = (
    row: SessionRow,
    run: (commands: DesktopCommands) => Promise<void>,
  ): void => {
    const commands = commandsFor(row.desktopId);
    void (commands ? run(commands) : Promise.reject(new Error("machine_unavailable")))
      .catch(() => Alert.alert(t("common.error")));
  };

  const renameSession = async (row: SessionRow, rawName: string) => {
    const name = rawName.trim();
    if (!name) return;
    const commands = commandsFor(row.desktopId);
    try {
      if (!commands) throw new Error("machine_unavailable");
      await commands.renameSession(row.session.sessionId, name);
    } catch {
      Alert.alert(t("common.error"));
    }
  };

  const openRename = (row: SessionRow) => {
    setRenameTarget(row);
    setRenameValue(row.session.title || "");
  };

  const submitRename = async () => {
    const row = renameTarget;
    const name = renameValue.trim();
    if (!row || !name) return;
    setRenameTarget(null);
    await renameSession(row, name);
  };

  const togglePin = (row: SessionRow) => {
    runCommand(row, commands =>
      commands.setSessionPinned(row.session.sessionId, row.session.threadId, !row.session.pinned));
  };

  const confirmDelete = (row: SessionRow) => {
    Alert.alert(
      t("sessions.delete"),
      t("sessions.deleteConfirm", {
        title: row.session.title || t("sessions.unnamed"),
      }),
      [
        { text: t("chat.cancel"), style: "cancel" },
        {
          text: t("sessions.delete"),
          style: "destructive",
          onPress: () => {
            runCommand(row, commands =>
              commands.deleteSession(row.session.sessionId, row.session.threadId));
          },
        },
      ],
    );
  };

  const openSessionMenu = (session: RemoteSession, desktopId: string | null) => {
    if (canCommand(desktopId)) setMenuSession({ session, desktopId });
  };

  const connection = remote.connectionPresentation;
  const connected = connection.level === "connected";
  const connecting = connection.level === "connecting";
  /**
   * Whether the conversation menu's actions can run: they are addressed to the
   * machine that owns the row, so that machine is what speaks for it.
   */
  const canManage = menuSession !== null && canCommand(menuSession.desktopId);
  /**
   * Every paired machine, the active one first.
   *
   * The status dot answers for the whole set, so the list the popover shows has
   * to hold the same machines the dot is judging: the one whose conversation is
   * open (read from the connection the app renders from) and every other
   * machine's own live connection.
   */
  const machines = useMemo<MachineSummary[]>(() => {
    if (!activeDesktopId) return catalogs.machines;
    const paired = remote.desktops.find(desktop => desktop.desktopId === activeDesktopId);
    return [
      {
        desktopId: activeDesktopId,
        label: paired?.name ?? activeDesktopId,
        ...(paired?.icon ? { icon: paired.icon } : {}),
        active: true,
        status: activeMachineStatus(connection),
      },
      ...catalogs.machines,
    ];
  }, [activeDesktopId, catalogs.machines, connection, remote.desktops]);
  /** What the user calls a paired machine; its id when they have not named it. */
  const desktopLabel = (desktopId: string): string =>
    machines.find(machine => machine.desktopId === desktopId)?.label ?? desktopId;
  /**
   * The workspaces the dialog offers: the picked machine's own list when the
   * phone is still on another one, else the connected machine's.
   */
  const workspaceOptions = deviceWorkspaces ?? remote.workspaces;
  /**
   * Whether this dialog may register a folder as a workspace.
   *
   * Creating one writes through the phone's active connection, so it belongs to
   * the machine the phone is on: a picked machine it is not on has no form to
   * open here, and says so by not offering the row (its own workspaces can
   * still be chosen — they are read from that machine).
   */
  const canCreateWorkspaceHere = remote.capabilities?.has("workspace_create_v1") === true
    && (newDeviceId === null || newDeviceId === activeDesktopId);
  /**
   * Open the creation dialog.
   *
   * Which machine the conversation belongs to is the first thing it asks — but
   * only when there is something to choose: with a single machine that can take
   * it the dialog is the one it always was, and on the conversations tab (where
   * nothing else is ever picked) the chat is still created straight away.
   */
  const openNew = () => {
    newDialogEpochRef.current += 1;
    // More than one machine can take the conversation — the same "can this
    // machine answer" judgement its rows' actions are offered under — so which
    // machine it is belongs to the user rather than to whichever one happens to
    // be connected: the dialog opens on that choice.
    const multiple = machines.filter(machine => canCommand(machine.desktopId)).length > 1;
    setNewDeviceStep(multiple);
    setNewDeviceId(null);
    setDeviceWorkspaces(null);
    setBusyDevice(null);
    setNewWorkspaceOpen(false);
    setNewWorkspaceError(null);
    if (tab === "chat") {
      // The conversations tab has nothing left to pick — create the chat
      // straight away without a dialog, unless the machine is that choice.
      setNewMode("chat");
      setWorkspaceId("");
      if (!multiple) {
        void remote.newConversation("chat");
        return;
      }
      setNewStep("device");
      setNewOpen(true);
      return;
    }
    setNewMode("workspace");
    setWorkspaceId(remote.workspaces[0]?.id ?? "");
    setNewStep(multiple ? "device" : "target");
    setNewOpen(true);
  };

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
    // "Reconnect" means every machine this phone holds, not just the one whose
    // conversation is open: the point of the button is to get the whole merged
    // list answering again.
    void catalogs.reconnect();
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
          title={menuSession?.session.title || t("sessions.unnamed")}
          visible={menuSession !== null}
          onClose={() => setMenuSession(null)}
          actions={menuSession ? [
            // A row on another machine acts there, and the sheet says so before
            // the first destructive item: the point of these commands is that
            // they are not local, and a delete that lands elsewhere without
            // saying so is the one action a reader cannot undo.
            ...(menuSession.desktopId && menuSession.desktopId !== activeDesktopId
              ? [{
                label: t("sessions.actionsOnDesktop", {
                  name: desktopLabel(menuSession.desktopId),
                }),
                heading: true,
              }]
              : []),
            { label: t(menuSession.session.pinned ? "sessions.unpin" : "sessions.pin"), icon: <Pin size={18} color={colors.inkSoft} />, disabled: !canManage, onPress: () => togglePin(menuSession) },
            { label: t("chat.rename"), icon: <Pencil size={18} color={colors.inkSoft} />, disabled: !canManage, onPress: () => openRename(menuSession) },
            { label: t("sessions.delete"), icon: <Trash2 size={18} color={colors.danger} />, destructive: true, disabled: !canManage, onPress: () => confirmDelete(menuSession) },
          ] : []}
        />
        <View style={styles.deviceBar}>
          <DesktopPicker
            desktops={remote.desktops}
            filter={desktopFilter}
            onChange={setDesktopFilter}
            onManage={onManageDesktops}
          />
          <View style={styles.topActions}>
            <ConnectionBadge
              active={active}
              machines={machines}
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
          commandsFor={commandsFor}
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
          <DialogSurface footer={newStep === "device" ? undefined : newWorkspaceOpen ? (
            <Button
              disabled={creatingWorkspace || !newWorkspacePath.trim()}
              label={t("sessions.createWorkspaceAction")}
              onPress={() => void submitNewWorkspace()}
            />
          ) : (
            <Button
              disabled={busyDevice !== null || (newMode === "workspace" && !workspaceId)}
              label={t("sessions.new")}
              onPress={() => void startConversation()}
            />
          )}>
              {newStep === "device" ? (
                <>
                  <View style={styles.dialogHeader}>
                    <Text accessibilityRole="header" style={styles.dialogTitle}>{t("sessions.chooseDevice")}</Text>
                    <Pressable accessibilityRole="button" accessibilityLabel={t("common.close")} onPress={closeNew} style={styles.settingsButton}>
                      <X color={colors.inkMuted} size={20} />
                    </Pressable>
                  </View>
                  <View style={styles.workspaceOptionsContent}>
                    {machines.map((machine) => {
                      const creatable = canCommand(machine.desktopId);
                      // The machine the phone is on needs no switch, so it is
                      // the one marked; every other row is a machine to move to.
                      const current = machine.desktopId === activeDesktopId;
                      return (
                        <Pressable
                          key={machine.desktopId}
                          accessibilityLabel={machine.label}
                          accessibilityRole="button"
                          accessibilityState={{ disabled: !creatable, selected: current }}
                          disabled={!creatable || busyDevice !== null}
                          onPress={() => void chooseDevice(machine.desktopId)}
                          style={[
                            styles.workspaceOption,
                            current && styles.workspaceOptionActive,
                            !creatable && styles.deviceOptionUnavailable,
                          ]}
                        >
                          <Text style={styles.deviceGlyph}>{iconGlyph(machine.icon)}</Text>
                          <Text numberOfLines={1} style={styles.workspaceOptionName}>
                            {machine.label}
                          </Text>
                          {busyDevice === machine.desktopId ? (
                            <ActivityIndicator color={colors.accent} size="small" />
                          ) : !creatable ? (
                            // A machine that cannot take the conversation says
                            // why instead of being quietly absent: it is still
                            // paired, and the reader is looking for it.
                            <Text style={styles.deviceStatus}>
                              {t(machineStatusKey(machine.status))}
                            </Text>
                          ) : current ? <Check color={colors.accent} size={16} /> : null}
                        </Pressable>
                      );
                    })}
                  </View>
                </>
              ) : newWorkspaceOpen ? (
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
                    {newDeviceStep ? (
                      <Pressable accessibilityRole="button" accessibilityLabel={t("common.back")} onPress={() => setNewStep("device")} style={styles.settingsButton}>
                        <ChevronLeft color={colors.inkMuted} size={20} />
                      </Pressable>
                    ) : null}
                    <Text numberOfLines={1} style={styles.dialogTitle}>
                      {newDeviceStep
                        ? t("sessions.newOnDevice", {
                          name: desktopLabel(newDeviceId ?? activeDesktopId ?? ""),
                        })
                        : t("sessions.new")}
                    </Text>
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
                            setWorkspaceId(workspaceOptions[0]?.id ?? "");
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
                      {workspaceOptions.map((workspace) => (
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
                          "Unsupported command" — and on being the machine the
                          phone is connected to, which is where that write
                          lands. */}
                      {canCreateWorkspaceHere && (
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
                      {workspaceOptions.length === 0 && (
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
          generationKey={renameTarget?.session.sessionId}
          onGenerate={renameTarget && canCommand(renameTarget.desktopId)
            ? () => commandsFor(renameTarget.desktopId)!.generateTitle(
              renameTarget.session.sessionId,
              i18n.language.startsWith("zh") ? "zh" : "en",
            )
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
  /** A machine the reader can see but not create on, in the machine picker. */
  deviceOptionUnavailable: { opacity: 0.6 },
  deviceGlyph: { fontSize: 18 },
  deviceStatus: { color: colors.inkMuted, fontSize: 13 },
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
