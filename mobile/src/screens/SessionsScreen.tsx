import {
  ChevronLeft,
  ChevronRight,
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
import { activeMachineStatus, machineStatusKey, machinesConnected, type MachineSummary } from "../remote/desktopStatus";
import { mergeSessions, type DesktopFilter } from "../remote/mergeSessions";
import type { DesktopCommands } from "../remote/desktopCommands";
import { iconGlyph } from "../remote/peerIcons";
import type { RemoteSession, RemoteWorkspace } from "../remote/types";
import { SessionList, type SessionListHandle } from "./SessionList";
import { SessionListActions } from "./SessionListActions";
import { listTargets, selectableTargets } from "./sessionSelection";
import { type Tab } from "./sessionTabs";
import { DesktopPicker } from "./DesktopPicker";
import { DisconnectedScreen } from "./DisconnectedScreen";
import { StandbyScreen } from "./StandbyScreen";
import { layout, radius, spacing } from "../theme/tokens";
import { createThemedStyles, useAppearance, useTheme } from "../theme/ThemeContext";
import { promptUpgrade } from "../update/prompt";
import { checkForUpdate } from "../update/update";
import { SettingsScreen, type SettingsScreenHandle } from "../features/settings/SettingsScreen";

/**
 * What the new-conversation dialog can create.
 *
 * A page of the list and a conversation's mode are different questions: the
 * recency page creates chats, but "recent" is not something a conversation can
 * be.
 */
type NewConversationMode = "workspace" | "chat";

/** A conversation row, with the machine that owns it. */
interface SessionRow {
  session: RemoteSession;
  /** The desktop the row came from; null for the ordinary single-machine list. */
  desktopId: string | null;
}

// Persist the selected tab across screen transitions (SessionsScreen is
// unmounted when entering a chat and remounted when returning).
let lastTab: Tab = "chat";

/**
 * The "every desktop" scope, as one shared object.
 *
 * A module constant rather than a fresh literal: the merged list is memoized on
 * the scope, and a new object on every render would re-merge every machine's
 * conversation catalogue on every render.
 */
const ALL_DESKTOPS: DesktopFilter = { kind: "all" };

function deferPresentation(action: () => void): void {
  // Let the native action sheet finish dismissing before presenting another
  // alert. InteractionManager can remain pending during native transitions.
  setTimeout(action, Platform.OS === "ios" ? 350 : 0);
}

export function SessionsScreen({ onManageDesktops, active = true }: {
  onManageDesktops(): void;
  active?: boolean;
}) {
  const styles = useStyles();
  const { colors } = useTheme();
  const { fontSize } = useAppearance();
  const Alert = useAppDialog(active);
  const { t, i18n } = useTranslation();
  const remote = useRemote();
  const [tab, setTabState] = useState<Tab>(lastTab);
  /**
   * Which desktops the session list shows. Kept for the life of the screen: a
   * filter that reset itself on every re-render would look like the list had a
   * mind of its own.
   */
  const [desktopFilter, setDesktopFilter] = useState<DesktopFilter>(ALL_DESKTOPS);
  const activeDesktopId = remote.credentials?.expectedDesktopId ?? null;
  /**
   * Every *other* paired machine's catalogue, read from that machine's own
   * runtime.
   *
   * Each of those machines keeps its connection open for as long as it is
   * paired, so this is a view of state that already exists rather than a second
   * set of connections owned by this screen. The merged list can therefore show
   * their conversations without disturbing the one being read — and reading a
   * row that belongs to another machine no longer costs a reconnection.
   */
  const catalogs = remote.catalogs;
  /**
   * The scope actually in force.
   *
   * A scope can name a machine that was unpaired while the list was showing it —
   * the picker's own "manage" page is one tap away — and a list standing on a
   * machine the picker no longer offers would be describing nothing at all. Such
   * a scope falls back to every desktop here rather than through an effect, so no
   * pass ever draws the list for a machine that is already gone.
   */
  const scopedFilter = desktopFilter.kind === "all"
    || remote.desktops.some(desktop => desktop.desktopId === desktopFilter.desktopId)
    ? desktopFilter
    : ALL_DESKTOPS;
  const mergedRows = useMemo(
    () => mergeSessions(catalogs.catalogs, scopedFilter),
    [catalogs.catalogs, scopedFilter],
  );
  /**
   * The machine the picker names that the phone is not reading, if any.
   *
   * It is the one case the ordinary workspace tree cannot draw: its rows come
   * from that machine's own runtime rather than from the connection the screens
   * are wired to — and a machine that is not answering has none at all.
   */
  const scopedPeerId = scopedFilter.kind === "desktop" && scopedFilter.desktopId !== activeDesktopId
    ? scopedFilter.desktopId
    : null;
  /**
   * The merged view replaces the workspace tree when the list is showing another
   * desktop, and when it holds rows the tree cannot show — a user with one
   * pairing keeps today's list exactly, because the flat view is a different
   * information architecture (a workspace name can exist on two machines at
   * once).
   *
   * A machine the picker names is always drawn from the merged rows, even when it
   * has none: the tree below belongs to the machine the phone is reading, and
   * under the picker's name that is one machine's conversations presented as
   * another's.
   */
  const showMerged = useMemo(
    () => scopedPeerId !== null || mergedRows.some(row => row.desktopId !== activeDesktopId),
    [mergedRows, activeDesktopId, scopedPeerId],
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
  const [newMode, setNewMode] = useState<NewConversationMode>("chat");
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
   * Reading another machine's workspaces is asynchronous and the user can close
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
  /**
   * The list, for the two modes its own header no longer carries.
   *
   * Search and batch selection are still the list's — they hold its rows' state
   * — while the header above it belongs to this screen. The handle is the way
   * in, the same shape the settings screen is driven by.
   */
  const listRef = useRef<SessionListHandle>(null);
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
   * Take the machine the user picked.
   *
   * A machine the phone is not on is read down *its own* connection before the
   * next step, because that machine's workspaces are what the picker must
   * offer — the connected machine's list names workspaces that machine has
   * never heard of. The creation itself waits until it is confirmed: browsing
   * the picker must not move the phone.
   */
  const chooseDevice = async (desktopId: string) => {
    if (busyDevice) return;
    const epoch = newDialogEpochRef.current;
    setNewDeviceId(desktopId);
    if (tab !== "workspace") {
      // Nothing else to pick for a chat, so the machine is the whole choice. The
      // recency page is not a workspace page either: "new" there means a chat,
      // which is the thing that has no workspace to choose.
      startConversationOn(desktopId, "chat", "");
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
    // Name the machine the pairing being removed belongs to: the active pair,
    // which is what `remote.unpair()` drops. `unpairConfirm` carries a
    // `{{name}}` placeholder, and omitting the option leaves the raw
    // placeholder in the sentence.
    Alert.alert(t("sessions.unpair"), t("sessions.unpairConfirm", {
      name: desktopLabel(activeDesktopId ?? ""),
    }), [
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
   * Create a conversation on a machine named by the user, reporting one that
   * cannot be reached at all.
   *
   * `newConversation` cannot say *where*: it is the open machine's verb, and a
   * screen's copy of it belongs to the render that drew the screen, so awaiting a
   * move and then calling it would build the draft on the machine the phone had
   * just left. The provider's desktop-addressed verb names the machine instead.
   */
  const createOn = (
    desktopId: string,
    mode: "chat" | "workspace",
    selectedWorkspaceId: string,
  ) => {
    void remote.newConversationOn(desktopId, mode, selectedWorkspaceId)
      .catch((error: unknown) => Alert.alert(t(
        error instanceof Error && error.message === "desktop_unavailable"
          ? "desktops.switchFailed"
          : "common.error",
      )));
  };

  /**
   * Open the new conversation on the machine the dialog named.
   *
   * The draft is local, but the conversation is not: it becomes a session on the
   * machine that will run it, which is the machine the dialog named — so the
   * draft is built *there* rather than on whichever machine the phone happens to
   * be on. `newConversation` cannot say that: it is the open machine's verb, and
   * this dialog's copy of it belongs to the render that drew the dialog, so
   * awaiting a move and then calling it would build the draft on the machine the
   * phone had just left. The provider's desktop-addressed verb names the machine,
   * builds the draft on it, and moves the phone onto it.
   *
   * Deferred until the Modal has released UIKit, because creating the draft is
   * what navigates to it.
   */
  const startConversationOn = (
    desktopId: string,
    mode: "chat" | "workspace",
    selectedWorkspaceId: string,
  ) => {
    const start = () => createOn(desktopId, mode, selectedWorkspaceId);
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

  const startConversation = () => {
    if (newMode === "workspace" && !workspaceId) return;
    // The machine the dialog named, defaulting to the one the phone is on —
    // which is also what it names when the machine step never appeared.
    const desktopId = newDeviceId ?? activeDesktopId;
    if (!desktopId) return;
    // The creation happens at the moment the user commits to it — browsing the
    // picker must not move the phone — and a machine that cannot be reached says
    // so instead of opening a draft that would be sent somewhere else.
    startConversationOn(desktopId, newMode, workspaceId);
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
        ...(remote.presence?.control ? { control: remote.presence.control } : {}),
      },
      ...catalogs.machines,
    ];
  }, [activeDesktopId, catalogs.machines, connection, remote.desktops, remote.presence]);
  /** What the user calls a paired machine; its id when they have not named it. */
  const desktopLabel = (desktopId: string): string =>
    machines.find(machine => machine.desktopId === desktopId)?.label ?? desktopId;
  /**
   * How the whole set of paired machines stands.
   *
   * `any` is the phone's own availability, and `scopeAnswers` says whether the
   * machine the list is *showing* is one of the ones answering. The list area and
   * the whole-page states below belong to the phone, and the phone holds several
   * desktops: one machine answering is enough for the merged list to be worth
   * drawing, while the machine whose conversation happens to be open is only one
   * of them. Reading those states from that one machine is how a reader with two
   * of three desktops up was handed a red "no connection" page over a list that
   * had rows to show.
   */
  const machineCounts = machinesConnected(machines);
  const scopeAnswers = scopedPeerId !== null
    ? peerReady.has(scopedPeerId)
    : scopedFilter.kind === "desktop"
      ? connected
      : machineCounts.any;
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
    // be connected: the dialog opens on that choice. The machine the phone reads
    // counts even when it cannot take one: a peer answering while it is down is
    // still a question of *which* machine, because a draft built on a machine
    // that is not answering is a draft addressed to nobody.
    const takesWork = machines.filter(machine => canCommand(machine.desktopId));
    const only = takesWork.length === 1 ? takesWork[0]?.desktopId ?? null : null;
    const multiple = takesWork.length > 1 || (only !== null && only !== activeDesktopId);
    // A machine the list is showing is already named by the picker, so the
    // dialog starts on it: a conversation started from this screen belongs to the
    // machine the screen is about, not to the one the phone happens to read. Its
    // workspaces are read from it in the machine step, as any named machine's
    // are.
    const named = scopedPeerId;
    setNewDeviceStep(multiple || named !== null);
    setNewDeviceId(named);
    setDeviceWorkspaces(null);
    setBusyDevice(null);
    setNewWorkspaceOpen(false);
    setNewWorkspaceError(null);
    if (tab !== "workspace") {
      // The conversations and recency pages have nothing left to pick, so a chat
      // is created straight away — on the machine the list is showing when it
      // names one, and otherwise without a dialog at all unless the machine is
      // that choice.
      setNewMode("chat");
      setWorkspaceId("");
      if (named !== null) {
        createOn(named, "chat", "");
        return;
      }
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
    setNewStep(multiple || named !== null ? "device" : "target");
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
  /**
   * Whether a machine other than the open one is answering.
   *
   * The whole-page state is the phone's own, and the phone holds several
   * desktops: one machine answering means the list below has rows to draw and
   * the reader has somewhere to work, so it cannot claim there is nothing to
   * connect to. A machine still connecting is not that machine: nothing is
   * readable from it yet, and the phone's own state is what the page reports.
   */
  const anotherMachineAnswers = machines.some(
    machine => machine.desktopId !== activeDesktopId && machine.status === "connected",
  );
  const showStandaloneStatus =
    !anotherMachineAnswers
    && (connection.level === "disconnected" || standaloneConnecting);
  /**
   * Whether the only machine this phone holds is in someone else's hands.
   *
   * A single pairing leaves nowhere else to read, and a taken-over desktop
   * refuses this list's own actions — every command answers null and sending is
   * blocked — so drawing the conversations first is a page the user cannot use.
   * The takeover is the only thing that changes that, and it used to wait behind
   * the status dot; with one machine the page asks for it directly. Several
   * pairings keep the list: those rows are still this phone's to read, and the
   * badge names the open machine's standby.
   */
  const onlyMachineHeld = remote.desktops.length === 1
    && connected
    && remote.presence?.control === "standby";

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

  // The workspace tree is the open machine's own, so its empty state answers for
  // that machine rather than for the set: saying "no workspaces" under one
  // machine's tree because *another* machine is up would name the wrong machine.
  const workspaceEmpty = !connected ? (
    offlineEmpty
  ) : (
    <Text style={styles.empty}>{t("sessions.noWorkspaces")}</Text>
  );

  /**
   * What the list says when the picker names another machine and there is
   * nothing to draw under it.
   *
   * A machine that is not answering has no rows of its own, and the reader still
   * has to be able to tell that apart from "this machine has no conversations
   * yet" — so the state names the machine, says how it stands, and offers the one
   * action that can change that, on that machine's own connection. Drawing the
   * active machine's list here instead is what made picking an offline desktop
   * look like the tap had been ignored.
   */
  const scopedPeerEmpty = (() => {
    if (scopedPeerId === null) return null;
    const machine = machines.find(item => item.desktopId === scopedPeerId);
    const status = machine?.status ?? "offline";
    if (status === "connected") {
      // A machine another device drives holds no list of this phone's: its rows
      // went when control left, because the host refuses this client every read
      // and sends it no snapshot. So the page asks for the one thing that
      // changes that, instead of reading "no conversations" over a machine full
      // of them.
      if (machine?.control === "standby")
        return (
          <StandbyScreen
            holder={machine.holder ?? null}
            onTakeControl={() => remote.catalogs.takeControlFor(scopedPeerId)}
          />
        );
      // Answering and simply empty: the ordinary copy for this tab, whose own
      // actions are already addressed to the machine the list is showing.
      return tab === "workspace"
        ? <Text style={styles.empty}>{t("sessions.noWorkspaces")}</Text>
        : createChatEmpty;
    }
    return (
      <View style={styles.emptyState}>
        <View
          style={[
            styles.emptyIcon,
            { backgroundColor: status === "offline" ? colors.dangerSoft : colors.warningSoft },
          ]}
        >
          <Unplug color={status === "offline" ? colors.danger : colors.warning} size={26} />
        </View>
        <Text style={styles.emptyTitle}>
          {`${iconGlyph(machine?.icon)} ${desktopLabel(scopedPeerId)}`}
        </Text>
        <Text style={styles.emptyHint}>{t(machineStatusKey(status))}</Text>
        <Text style={styles.emptyHint}>{t("desktops.unreachableHint")}</Text>
        <Button
          compact
          label={t("connection.reconnect")}
          onPress={() => void remote.reconnect(scopedPeerId)}
        />
      </View>
    );
  })();

  /**
   * Whether a new conversation can be started for what the list is showing.
   *
   * With every desktop in view the dialog asks which machine, so the active
   * machine's own connection is enough; with one machine named it is *that*
   * machine's answer that decides — the same judgement its rows' actions are
   * offered under, so the button never promises a conversation on a machine the
   * list is describing as unreachable.
   */
  const canStartConversation = scopedPeerId === null ? scopeAnswers : canCommand(scopedPeerId);
  /**
   * Whether the header gives the machine its own line.
   *
   * At the largest text size the name no longer fits beside the list's tools,
   * the status dot and settings on a phone-sized width — measured at 20px text
   * on a 360pt phone, "实验室的 MacBook Pro" needs about 340pt of line and 140pt
   * is left — and the name is the one thing here that must stay whole. So the
   * machine takes a line of its own and the tools move under it, which is the
   * shape this header had before its two rows were merged.
   */
  const stackHeader = fontSize === "xlarge";
  /**
   * Whether entering a batch selection has anything to hold.
   *
   * Read through the same two functions the list selects rows by — the list's
   * rows, then the machines that can answer a delete — so the header's item
   * cannot offer a selection the list would refuse to make. It is the list's
   * *catalogue* rather than its drawn rows: a folded group's conversations are
   * still selectable, which is why the list answers this question the same way.
   */
  const canSelect = selectableTargets(
    listTargets(showMerged ? mergedRows : undefined, remote.sessions),
    commandsFor,
  ).length > 0;

  return (
    <SafeAreaView style={styles.safe}>
      <View style={styles.page}>
        {Alert.dialog}
        <ActionMenu
          title={menuSession?.session.title || t("sessions.unnamed")}
          visible={menuSession !== null}
          onClose={() => setMenuSession(null)}
          actions={menuSession ? [
            // Every row's sheet names the machine its commands are addressed to,
            // the one the phone is reading included: they are all that machine's
            // commands, and a delete that lands there without the sheet saying
            // which machine is the one action a reader cannot undo. Naming only
            // a peer's row left the shape of a sheet depending on which machine
            // the phone happened to be reading — the same conversation gained and
            // lost this line as the reader moved.
            ...(menuSession.desktopId
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
        <View style={[styles.deviceBar, stackHeader && styles.deviceBarStacked]}>
          <DesktopPicker
            desktops={remote.desktops}
            filter={scopedFilter}
            machines={machines}
            onChange={setDesktopFilter}
            onManage={onManageDesktops}
            onTakeControl={remote.catalogs.takeControlFor}
            fullWidth={stackHeader}
          />
          <SessionListActions
            canSelect={canSelect}
            onSearch={() => listRef.current?.startSearch()}
            onSelect={() => listRef.current?.startSelecting()}
          />
          <View style={styles.topActions}>
            <ConnectionBadge
              active={active}
              machines={machines}
              presentation={connection}
              disconnectReason={remote.presence?.reason}
              error={remote.error}
              holder={remote.presence?.holder ?? null}
              onTakeControl={remote.takeControl}
              standby={remote.presence?.control === "standby"}
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
        ) : onlyMachineHeld ? (
          <StandbyScreen
            holder={remote.presence?.holder ?? null}
            onTakeControl={remote.takeControl}
          />
        ) : <SessionList
          ref={listRef}
          key={tab}
          tab={tab}
          active={active}
          onTabChange={setTab}
          activeDesktopId={remote.credentials?.expectedDesktopId ?? null}
          icons={peerIcons}
          commandsFor={commandsFor}
          merged={showMerged ? mergedRows : undefined}
          empty={scopedPeerEmpty ?? (tab === "workspace" ? workspaceEmpty : !scopeAnswers ? offlineEmpty : createChatEmpty)}
          onMenu={openSessionMenu}
        />}

        {canStartConversation && !onlyMachineHeld && (
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
                  {/* Machines to go *into*, not options to tick: a tap takes
                      the dialog to that machine's own next step — its
                      workspaces, or straight to the chat it creates in the
                      conversations tab — so every row carries the chevron a row
                      that opens something always has, and none of them is
                      pre-marked. A tick on the machine the phone is already on
                      read as "this one is chosen", which offered the reader a
                      selection to confirm when the tap *is* the confirmation. */}
                  <View style={styles.workspaceOptionsContent}>
                    {machines.map((machine) => {
                      const creatable = canCommand(machine.desktopId);
                      return (
                        <Pressable
                          key={machine.desktopId}
                          accessibilityLabel={machine.label}
                          accessibilityRole="button"
                          accessibilityState={{ disabled: !creatable }}
                          disabled={!creatable || busyDevice !== null}
                          onPress={() => void chooseDevice(machine.desktopId)}
                          style={[
                            styles.workspaceOption,
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
                          ) : (
                            <ChevronRight color={colors.inkMuted} size={18} />
                          )}
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
                    {(["workspace", "chat"] as NewConversationMode[]).map((mode) => (
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
                      {/* The phone can't browse the host, so it registers a
                          directory the user typed — the same store write as the
                          Desktop's own create-workspace dialog. Gated on the
                          capability: an older desktop answers the command with
                          "Unsupported command" — and on being the machine the
                          phone is connected to, which is where that write
                          lands. It leads the list, not trails it: the body
                          scrolls, so on a machine with many workspaces the row
                          was only reachable after scrolling past all of them. */}
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

const useStyles = createThemedStyles(theme => StyleSheet.create({
  deviceBar: {
    flexDirection: "row",
    alignItems: "center",
    gap: spacing.sm,
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
    backgroundColor: theme.colors.accentSoft,
  },
  desktopIdentity: { flex: 1, minWidth: 0, color: theme.colors.ink, fontSize: theme.font(15), fontWeight: "600" },
  safe: { flex: 1, backgroundColor: theme.colors.surface },
  page: { flex: 1, width: "100%", maxWidth: layout.contentMaxWidth, alignSelf: "center", backgroundColor: theme.colors.surface },
  // A line of its own for the machine when the reader's text size says so; the
  // tools wrap under it, still inside this row (see `stackHeader`).
  deviceBarStacked: { flexWrap: "wrap" },
  topActions: { flexDirection: "row", alignItems: "center", gap: spacing.xs, marginLeft: "auto" },
  settingsButton: {
    width: 44,
    height: 44,
    alignItems: "center",
    justifyContent: "center",
    borderRadius: radius.md,
  },
  pressed: { backgroundColor: theme.colors.surfaceSubtle },
  empty: { color: theme.colors.inkMuted, fontSize: theme.font(14) },
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
  emptyIconOffline: { backgroundColor: theme.colors.dangerSoft },
  emptyIconIdle: { backgroundColor: theme.colors.accentSoft },
  emptyTitle: {
    alignSelf: "stretch",
    textAlign: "center",
    color: theme.colors.ink,
    fontSize: theme.font(16),
    fontWeight: "700",
  },
  emptyHint: {
    marginBottom: spacing.md,
    alignSelf: "stretch",
    textAlign: "center",
    color: theme.colors.inkMuted,
    fontSize: theme.font(13),
    lineHeight: theme.leading(19),
  },
  emptyInside: {
    paddingHorizontal: spacing.md,
    paddingVertical: spacing.sm,
    color: theme.colors.inkMuted,
    fontSize: theme.font(13),
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
    backgroundColor: theme.colors.accent,
    shadowColor: theme.colors.inkStrong,
    shadowOpacity: 0.2,
    shadowRadius: 8,
    elevation: 5,
  },
  fabPressed: { opacity: 0.8 },
  dialogHeader: { flexDirection: "row", alignItems: "center", justifyContent: "space-between" },
  dialogTitle: { flexShrink: 1, color: theme.colors.inkStrong, fontSize: theme.font(20), fontWeight: "700" },
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
    borderColor: theme.colors.line,
    borderRadius: radius.md,
  },
  modeOptionActive: { borderColor: theme.colors.accent, backgroundColor: theme.colors.accentSoft },
  modeOptionText: { flexShrink: 1, color: theme.colors.ink, fontSize: theme.font(14), fontWeight: "600" },
  workspaceOptionsContent: { gap: spacing.xs },
  workspaceOption: {
    minHeight: layout.touchTarget,
    flexDirection: "row",
    alignItems: "center",
    gap: spacing.sm,
    padding: spacing.sm,
    borderWidth: 1,
    borderColor: theme.colors.lineSoft,
    borderRadius: radius.sm,
  },
  workspaceOptionActive: { borderColor: theme.colors.accent, backgroundColor: theme.colors.accentSoft },
  workspaceOptionName: { flex: 1, color: theme.colors.ink, fontSize: theme.font(14), fontWeight: "600" },
  workspaceForm: { gap: spacing.xs },
  /** A machine the reader can see but not create on, in the machine picker. */
  deviceOptionUnavailable: { opacity: 0.6 },
  deviceGlyph: { fontSize: theme.font(18) },
  deviceStatus: { color: theme.colors.inkMuted, fontSize: theme.font(13) },
  fieldLabel: { color: theme.colors.ink, fontSize: theme.font(14), fontWeight: "600" },
  workspaceInput: {
    minHeight: layout.touchTarget,
    paddingHorizontal: spacing.md,
    paddingVertical: spacing.sm,
    borderWidth: 1,
    borderColor: theme.colors.line,
    borderRadius: radius.md,
    color: theme.colors.ink,
    fontSize: theme.font(15),
  },
  fieldHint: { color: theme.colors.inkMuted, fontSize: theme.font(13), lineHeight: theme.leading(18) },
  fieldError: { color: theme.colors.danger, fontSize: theme.font(13), lineHeight: theme.leading(18) },
  settingsLabel: {
    color: theme.colors.inkMuted,
    fontSize: theme.font(12),
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
  updateVersion: { color: theme.colors.inkMuted, fontSize: theme.font(13) },
  approvalOptions: { flexDirection: "row", gap: spacing.sm },
  tierTriggerDisabled: { backgroundColor: theme.colors.surfaceSubtle, opacity: 0.55 },
}));
