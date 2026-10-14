import {
  Check,
  CheckCheck,
  ChevronDown,
  ChevronRight,
  CircleAlert,
  Folder,
  ListChecks,
  Minus,
  MoreHorizontal,
  Pencil,
  Pin,
  Plus,
  Search,
  Trash2,
  X,
} from "lucide-react-native";
import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  ActivityIndicator,
  Animated,
  BackHandler,
  FlatList,
  Pressable,
  StyleSheet,
  Text,
  TextInput,
  View,
} from "react-native";
import type { ReactNode } from "react";
import { ActionMenu } from "../components/ActionMenu";
import { useAppDialog } from "../components/useAppDialog";
import { RenameModal } from "../features/chat/components/RenameModal";
import { showToast } from "../features/chat/utils";
import { useRemoteControls as useRemote } from "../remote/RemoteContext";
import type { DesktopCommands } from "../remote/desktopCommands";
import { effectiveRunStatus } from "../remote/sessionStatus";
import type { RemoteSession } from "../remote/types";
import { colors, layout, radius, spacing } from "../theme/tokens";
import { catalogRows, mergedRows as sessionTreeMergedRows, type CatalogRow } from "./sessionTree";
import { useCollapsedWorkspaces } from "./useCollapsedWorkspaces";
import { useSessionListScroll } from "./useSessionListScroll";
import { useTabSwipe, type PageDirection } from "./useTabSwipe";

// Keep navigation state when the screen unmounts to open a conversation.
// Workspace folds are persisted separately (they survive a restart too).
let savedExpanded = new Set<string>();

// The list's two pages, in the order the toolbar's tab control shows them and
// the horizontal swipe reads them: workspaces are the left page, conversations
// the right one.
const TABS = ["workspace", "chat"] as const;
type Tab = (typeof TABS)[number];

/** The page beside `tab` in a swipe direction, or null at either end of the row. */
function adjacentTab(tab: Tab, direction: PageDirection): Tab | null {
  const index = TABS.indexOf(tab) + (direction === "next" ? 1 : -1);
  return TABS[index] ?? null;
}

// Column geometry, mirroring the desktop rail (see docs/internals/desktop/PRODUCT.md §5.2):
// every row is [toggle column 16][gap 4][title], and a child row's start is its
// parent's title column — so a parent title and its children's titles line up.
// Leaves carry no toggle column and start at the list inset.
const ROW_INSET = 8;
const TOGGLE_WIDTH = 16;
const TOGGLE_GAP = 4;
const LEVEL_INDENT = TOGGLE_WIDTH + TOGGLE_GAP;
// Restores the 44x44 touch target around the 16px toggle without letting it
// reach the title column: 16 back into the list gutter + 4 to its right.
const TOGGLE_SLOP_LEFT = layout.touchTarget - TOGGLE_WIDTH - TOGGLE_GAP;
const TOGGLE_SLOP_RIGHT = TOGGLE_GAP;

/** Row start for a session at `depth` (deeper history flattens to three levels). */
function depthInset(depth: number): number {
  return ROW_INSET + Math.min(depth, 3) * LEVEL_INDENT;
}

/** A conversation row, as the batch and the row menus handle it. */
type SessionRow = Extract<CatalogRow, { kind: "session" }>;
/** A workspace heading row. */
type WorkspaceRow = Extract<CatalogRow, { kind: "workspace" }>;

/**
 * One conversation a batch can address: the key a tick refers to, what it is,
 * and the machine that owns it.
 */
interface BatchTarget {
  key: string;
  session: RemoteSession;
  desktopId: string | null;
}

/** A batch's rows, grouped by the machine that will delete them. */
function groupByMachine(targets: BatchTarget[]): Map<string | null, BatchTarget[]> {
  const byMachine = new Map<string | null, BatchTarget[]>();
  for (const target of targets) {
    const group = byMachine.get(target.desktopId);
    if (group) group.push(target);
    else byMachine.set(target.desktopId, [target]);
  }
  return byMachine;
}

export function SessionList({
  tab,
  active = true,
  onTabChange,
  empty,
  merged,
  icons,
  activeDesktopId = null,
  commandsFor,
  onMenu,
}: {
  tab: Tab;
  active?: boolean;
  onTabChange: (tab: Tab) => void;
  empty: ReactNode;
  /**
   * When set, the list shows these rows (every desktop's sessions merged)
   * instead of the active desktop's workspace tree.
   *
   * An empty array is a list with nothing to draw, not an absent one: the screen
   * that named a machine with no rows of its own says so through `empty`, and
   * falling back to the active machine's tree here would put one machine's
   * conversations under another machine's name.
   */
  merged?: import("../remote/mergeSessions").MergedSessionRow[];
  /** Per-desktop glyph, so a merged row shows which machine it is on. */
  icons?: Map<string, string>;
  /**
   * The desktop this screen is connected to. It is what tells a row of the
   * active machine from a row of another one — which decides where a tap and a
   * menu action have to be addressed.
   */
  activeDesktopId?: string | null;
  /**
   * The commands one machine answers, or null when it cannot answer any.
   *
   * Every action a row offers goes to the machine that owns it, and this is the
   * single source of truth for both halves of that: the object is what a menu
   * dispatches through, and null is why its items are shown unavailable. So a
   * machine the phone has not reached, or whose agent is restarting, offers
   * nothing that could only fail on the wire — and the list needs no second
   * notion of "reachable" that could disagree with the dispatch.
   */
  commandsFor: (desktopId: string | null) => DesktopCommands | null;
  /** The row's own machine accompanies it, so the action knows where to go. */
  onMenu: (session: RemoteSession, desktopId: string | null) => void;
}) {
  const remote = useRemote();
  const Alert = useAppDialog(active);
  const { t } = useTranslation();
  const [query, setQuery] = useState("");
  const [searching, setSearching] = useState(false);
  const listKey = `${remote.credentials?.pairId ?? ""}:${tab}`;
  const { listRef, initialOffset, onLayout, onContentSizeChange, onScrollBeginDrag, onScroll } =
    useSessionListScroll<CatalogRow>(query.trim() ? null : listKey);
  const [menuWorkspace, setMenuWorkspace] = useState<WorkspaceRow | null>(null);
  /** The group whose name the rename dialog is editing, and its draft. */
  const [renamingWorkspace, setRenamingWorkspace] = useState<WorkspaceRow | null>(null);
  const [workspaceName, setWorkspaceName] = useState("");
  const [wasActive, setWasActive] = useState(active);
  if (wasActive !== active) {
    setWasActive(active);
    if (!active) {
      setMenuWorkspace(null);
      setRenamingWorkspace(null);
    }
  }
  const { collapsed, toggleWorkspaceCollapsed } = useCollapsedWorkspaces();
  const [expanded, setExpanded] = useState(savedExpanded);
  useEffect(() => {
    savedExpanded = expanded;
  }, [expanded]);
  const [selecting, setSelecting] = useState(false);
  /**
   * The rows a batch holds, by row key.
   *
   * A key and not a session id: the merged list shows two machines at once, and
   * the same session id exists on both — keying by id would tick a row on one
   * machine when the user ticked the other's. In the ordinary single-machine
   * list a session row's key *is* its id, so nothing changes there.
   */
  const [selected, setSelected] = useState(new Set<string>());
  const [pressedSessionId, setPressedSessionId] = useState<string | null>(null);
  const [deleting, setDeleting] = useState(false);
  const deletingRef = useRef(false);
  /**
   * The machine that owns a row: the one this phone is talking to for a row of
   * its own, and the row's own source for a row merged in from elsewhere.
   */
  const rowCommands = (row: CatalogRow): DesktopCommands | null =>
    commandsFor(row.desktopId ?? null);
  const rows = useMemo(
    () => (merged !== undefined
      ? sessionTreeMergedRows(merged, query, activeDesktopId, tab, collapsed, expanded)
      : catalogRows(remote.sessions, remote.workspaces, tab, collapsed, expanded, query)),
    [remote.sessions, remote.workspaces, merged, tab, collapsed, expanded, query, activeDesktopId],
  );
  /**
   * Whether a row's own machine has to be named on it.
   *
   * Only when the list really holds more than one machine's conversations: with
   * a single desktop (the picker's filter, or one pairing) every glyph would
   * repeat what the selector above the list already says, and take the width the
   * title needs.
   */
  const showSource = useMemo(() => {
    const sources = new Set(
      rows.flatMap(row => (row.kind === "session" && row.desktopId ? [row.desktopId] : [])),
    );
    return sources.size > 1;
  }, [rows]);
  /**
   * Every conversation the list holds, folded or not, under the key a batch
   * uses for it.
   *
   * The batch cannot be resolved against the drawn rows: a folded group's rows
   * leave the list while a selection made before the fold still has to be
   * deleted, and a "select all in this workspace" is exactly the action that
   * reaches conversations the reader cannot see. In the ordinary single-machine
   * list a conversation's key is its own id, which is what the batch always
   * used.
   */
  const allSessions = useMemo<BatchTarget[]>(() => {
    if (merged !== undefined)
      return merged.map(row => ({
        key: row.key,
        session: row.session,
        desktopId: row.desktopId,
      }));
    return remote.sessions.map(session => ({
      key: session.sessionId,
      session,
      desktopId: null,
    }));
  }, [merged, remote.sessions]);
  // What a batch may hold: conversations that are not pinned shortcuts, on a
  // machine that can take the command. Pinned sessions are promoted above the
  // groups (and, on the workspace tab, out of their own workspace), so they
  // carry no checkbox while selecting and select-all never sweeps them up. A
  // machine that cannot answer is left out for the same reason its menu items
  // are: a checkbox would promise a delete it would refuse.
  //
  // Computed every render rather than memoized: it reads the machines through
  // `commandsFor`, whose answer can change while that callback's identity stays
  // put — a memo keyed on the callback would keep offering a checkbox after the
  // machine behind it went away.
  const selectableRows = rows.filter(
    (row): row is SessionRow =>
      row.kind === "session"
      && !row.session.pinned
      && commandsFor(row.desktopId ?? null) !== null,
  );
  /** The conversations the batch holds, folded away or not. */
  const targets = allSessions.filter(target => selected.has(target.key));
  const allSelected =
    selectableRows.length > 0
    && selectableRows.every(row => selected.has(row.key));
  /**
   * Whether any machine in the batch can carry the delete.
   *
   * A batch can hold several machines' conversations, so the action is offered
   * while at least one of them answers — the rest stay selected and reported —
   * and refused only when none does, which is a delete the sheet could not
   * carry out at all.
   */
  const canDeleteSelected = targets.some(target => commandsFor(target.desktopId) !== null);

  useEffect(() => {
    if (!active || (!selecting && !searching)) return;
    const subscription = BackHandler.addEventListener("hardwareBackPress", () => {
      if (!deletingRef.current) {
        if (selecting) {
          setSelecting(false);
          setSelected(new Set());
        } else {
          setSearching(false);
          setQuery("");
        }
      }
      return true;
    });
    return () => subscription.remove();
  }, [active, selecting, searching]);

  // Swiping the list pages it: left from workspaces into conversations, right
  // back out of them. Ignored while the toolbar is searching or selecting — the
  // tab bar is hidden then, so the swipe would change mode unseen.
  const swipe = useTabSwipe({
    enabled: !searching && !selecting,
    hasPage: direction => adjacentTab(tab, direction) !== null,
    onSwitch: direction => {
      const next = adjacentTab(tab, direction);
      if (next) onTabChange(next);
    },
  });

  const toggleSelection = (id: string) =>
    setSelected(current => {
      const next = new Set(current);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  const toggleFold = (id: string, workspace: boolean) => {
    if (workspace) {
      toggleWorkspaceCollapsed(id);
      return;
    }
    const next = new Set(expanded);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    setExpanded(next);
  };
  const deleteSelected = () => {
    if (!targets.length || !canDeleteSelected || deletingRef.current) return;
    Alert.alert(
      t("sessions.deleteSelected"),
      t("sessions.deleteSelectedConfirm", { count: targets.length }),
      [
        { text: t("chat.cancel"), style: "cancel" },
        {
          text: t("sessions.delete"),
          style: "destructive",
          onPress: () => {
            if (deletingRef.current) return;
            deletingRef.current = true;
            setDeleting(true);
            void (async () => {
              const failed = new Set<string>();
              // One machine's conversations at a time, one conversation at a
              // time: a selection can hold several machines' rows, and each
              // machine answers only for its own. Sequential requests avoid
              // flooding a desktop; failures stay selected for retry.
              const byMachine = groupByMachine(targets);
              for (const [desktopId, group] of byMachine) {
                const commands = commandsFor(desktopId);
                // A machine that went away between ticking the boxes and
                // deleting: its rows stay selected, so the retry is one tap
                // rather than a re-selection.
                if (!commands) {
                  for (const row of group) failed.add(row.key);
                  continue;
                }
                for (const row of group) {
                  try {
                    await commands.deleteSession(row.session.sessionId, row.session.threadId);
                  } catch {
                    failed.add(row.key);
                  }
                }
              }
              setSelected(failed);
              setSelecting(failed.size > 0);
              deletingRef.current = false;
              setDeleting(false);
              if (failed.size)
                Alert.alert(
                  t("common.error"),
                  t("sessions.deletePartialFailure", { count: failed.size }),
                );
            })();
          },
        },
      ],
    );
  };

  /**
   * Every conversation under one heading.
   *
   * Read from the catalogue rather than from the drawn list, so a fold cannot
   * hide rows from a "select all in this workspace" — the point of the action is
   * the ones the reader cannot see. Pinned sessions belong to the shortcuts
   * above the groups instead, and a machine that cannot delete a row does not
   * get it: the batch is addressed to the group's own machine, so rows it would
   * refuse are left out here rather than failing later.
   */
  const sessionsInWorkspace = (workspace: WorkspaceRow): BatchTarget[] =>
    allSessions.filter(target =>
      target.session.mode === "workspace"
      && !target.session.pinned
      && (target.session.workspaceId ?? "") === workspace.workspaceId
      && (target.desktopId ?? null) === (workspace.desktopId ?? null)
      && commandsFor(target.desktopId) !== null);

  /**
   * Start a conversation in a workspace, on the machine that owns it.
   *
   * The workspace says which machine runs it, so that machine is named rather
   * than reached by moving the phone first: `newConversation` is the *open*
   * machine's verb, and this screen's copy of it was built from the render that
   * drew the screen, so awaiting a switch and then calling it would open the
   * draft on the machine the phone had just left. The provider names the machine
   * instead (it reads that machine's runtime out of its registry), builds the
   * draft there, and moves the phone onto it — which is what the reader asked
   * for, and leaves them looking at the draft they just created.
   *
   * This machine's own group needs no name: its own verb is the open machine's,
   * and there is nothing to move.
   *
   * Offering the item for the connected machine only (as this did) made the same
   * sheet gain and lose "new conversation" as the phone's connection followed the
   * reader between desktops: the group they had just been reading had it, every
   * other group in the same list did not.
   */
  const startConversationInWorkspace = async (workspace: WorkspaceRow) => {
    const machine = workspace.desktopId && workspace.desktopId !== activeDesktopId
      ? workspace.desktopId
      : null;
    try {
      if (machine) await remote.newConversationOn(machine, "workspace", workspace.workspaceId);
      else await remote.newConversation("workspace", workspace.workspaceId);
    }
    catch (error) {
      // A machine that cannot be reached is the one failure worth wording
      // differently: nothing was created anywhere, and the reader was told the
      // machine, not the creation, is what failed.
      Alert.alert(t(error instanceof Error && error.message === "desktop_unavailable"
        ? "desktops.switchFailed"
        : "common.error"));
    }
  };

  const selectWorkspaceSessions = (workspace: WorkspaceRow) => {
    const keys = sessionsInWorkspace(workspace).map(row => row.key);
    if (keys.length === 0) return;
    setSelecting(true);
    setSelected(current => new Set([...current, ...keys]));
  };

  const confirmDeleteWorkspace = (workspace: WorkspaceRow) => {
    if (deletingRef.current) return;
    Alert.alert(
      t("sessions.deleteWorkspace"),
      t("sessions.deleteWorkspaceConfirm", {
        title: workspace.workspace.name || t("sessions.workspace"),
        count: workspace.count,
      }),
      [
        { text: t("chat.cancel"), style: "cancel" },
        {
          text: t("sessions.delete"),
          style: "destructive",
          onPress: () => {
            const commands = commandsFor(workspace.desktopId ?? null);
            if (deletingRef.current || !commands) {
              // The machine went away while the confirmation was up: the host
              // still holds the workspace, and saying so is what keeps the
              // heading from looking deleted until the next refresh.
              if (!commands) Alert.alert(t("common.error"), t("sessions.deleteWorkspaceFailed"));
              return;
            }
            deletingRef.current = true;
            setDeleting(true);
            // The workspace's conversations go with it, so a selection that held
            // any of them must not keep rows the host no longer has.
            const removed = new Set(
              allSessions
                .filter(target =>
                  (target.session.workspaceId ?? "") === workspace.workspaceId
                  && (target.desktopId ?? null) === (workspace.desktopId ?? null))
                .map(target => target.key),
            );
            void commands
              .deleteWorkspace(workspace.workspaceId)
              .then(() => {
                setSelected(current => {
                  const next = new Set(current);
                  for (const key of removed) next.delete(key);
                  return next;
                });
              })
              .catch(() => Alert.alert(t("common.error"), t("sessions.deleteWorkspaceFailed")))
              .finally(() => {
                deletingRef.current = false;
                setDeleting(false);
              });
          },
        },
      ],
    );
  };

  const openWorkspaceMenu = (workspace: WorkspaceRow) => {
    if (!commandsFor(workspace.desktopId ?? null) || deleting) return;
    setMenuWorkspace(workspace);
  };

  /** Rename the workspace on the machine that owns it. The draft is presented
   * already filled with the current name; the host's accepted name comes back
   * through its catalogue, so the group's heading follows it. */
  const renameWorkspace = async (workspace: WorkspaceRow, rawName: string) => {
    const name = rawName.trim();
    if (!name) return;
    try {
      const commands = commandsFor(workspace.desktopId ?? null);
      if (!commands) throw new Error("machine_unavailable");
      await commands.renameWorkspace(workspace.workspaceId, name);
    } catch {
      Alert.alert(t("common.error"), t("sessions.renameWorkspaceFailed"));
    }
  };

  const submitWorkspaceRename = async () => {
    const workspace = renamingWorkspace;
    const name = workspaceName.trim();
    if (!workspace || !name) return;
    setRenamingWorkspace(null);
    await renameWorkspace(workspace, name);
  };

  const hostName = (desktopId: string | null | undefined) => {
    if (!desktopId) return "";
    return remote.desktops.find(desktop => desktop.desktopId === desktopId)?.name ?? desktopId;
  };
  /** What the open workspace menu acts through — null when it cannot act. */
  const menuWorkspaceCommands = menuWorkspace ? rowCommands(menuWorkspace) : null;

  const renderRow = ({ item }: { item: CatalogRow }) => {
    /**
     * Whether this row belongs to a machine other than the connected one.
     *
     * A merged row carries its source; the ordinary list's rows do not (every
     * row there is the active desktop's).
     */
    const foreign = !!item.desktopId && item.desktopId !== activeDesktopId;
    const commands = rowCommands(item);
    if (item.kind === "workspace") {
      const isCollapsed = collapsed.has(item.workspace.id) && !query.trim();
      const name = item.workspace.name || t("sessions.workspace");
      // A group on another machine belongs to that machine, and its actions —
      // rename, pin, delete — are that machine's commands, so the heading offers
      // them there whenever it can answer. Starting a conversation in it is that
      // machine's too, and the heading offers it there: the draft is built on the
      // machine that runs it (see `startConversationInWorkspace`), rather than the
      // group being left without the item the group beside it has.
      return (
        <View style={styles.workspace}>
          <Pressable
            accessibilityRole="button"
            accessibilityLabel={name}
            accessibilityState={{ expanded: !isCollapsed }}
            onPress={() => toggleFold(item.workspace.id, true)}
            style={({ pressed }) => [styles.workspaceBody, pressed && styles.pressed]}
          >
            {isCollapsed ? (
              <ChevronRight size={16} color={colors.inkSoft} />
            ) : (
              <ChevronDown size={16} color={colors.inkSoft} />
            )}
            <Folder size={16} color={colors.accent} />
            <Text numberOfLines={1} style={styles.workspaceName}>
              {name}
            </Text>
            {showSource && icons?.get(item.desktopId ?? "") ? (
              <Text accessibilityLabel={item.desktopId} style={styles.sourceIcon}>
                {icons.get(item.desktopId ?? "")}
              </Text>
            ) : null}
            {item.workspace.pinned && <Pin size={13} color={colors.accent} />}
            <Text style={styles.count}>{item.count}</Text>
          </Pressable>
          <Pressable
            accessibilityRole="button"
            accessibilityLabel={t("sessions.workspaceActions", { title: name })}
            accessibilityState={{ disabled: deleting || !commands }}
            disabled={deleting || !commands}
            onPress={() => openWorkspaceMenu(item)}
            style={[styles.iconButton, (deleting || !commands) && styles.disabled]}
          >
            <MoreHorizontal size={18} color={colors.inkMuted} />
          </Pressable>
        </View>
      );
    }
    const session = item.session;
    const checked = selected.has(item.key);
    // A pinned session is a shortcut outside the tab's own groups: it keeps no
    // checkbox and cannot join the batch, so select-all never includes it. Nor
    // can a conversation on a machine that cannot answer the delete: a checkbox
    // would promise a removal that machine would refuse.
    const selectable = !session.pinned && commands !== null;
    const status = effectiveRunStatus(session.status, session.streaming);
    const running = status === "running" || status === "queued";
    const unread = remote.unreadSessions.has(session.sessionId);
    return (
      <View style={[styles.row, { paddingLeft: depthInset(item.depth) }, pressedSessionId === session.sessionId && styles.rowPressed]}>
        {selecting
          ? selectable
            ? (
                <Pressable
                  accessibilityRole="checkbox"
                  accessibilityLabel={session.title || t("sessions.unnamed")}
                  accessibilityState={{ checked, disabled: deleting }}
                  disabled={deleting}
                  onPress={() => toggleSelection(item.key)}
                  style={styles.iconButton}
                >
                  <View style={[styles.checkbox, checked && styles.checkboxChecked]}>
                    {checked && <Check size={13} color={colors.surface} />}
                  </View>
                </Pressable>
              )
            // A pinned shortcut keeps the checkbox column's width (no box) so
            // its title stays on the column the group rows beside it use.
            : <View style={styles.iconButton} />
          : null}
        {item.hasChildren ? (
          <Pressable
            accessibilityRole="button"
            accessibilityLabel={t(
              expanded.has(item.foldId)
                ? "sessions.collapseChildren"
                : "sessions.expandChildren",
            )}
            accessibilityState={{ expanded: expanded.has(item.foldId) }}
            onPress={() => toggleFold(item.foldId, false)}
            // The toggle is laid out 16px wide (TOGGLE_WIDTH) so a child's row
            // start lands on its parent's title column, exactly as on desktop.
            // hitSlop then restores the 44x44 touch target: 16 into the list's
            // own gutter on the left, 4 on the right — which stops exactly at
            // the title column, so it never swallows title taps.
            hitSlop={{ left: TOGGLE_SLOP_LEFT, right: TOGGLE_SLOP_RIGHT }}
            style={styles.expander}
          >
            {/* A +/− tree toggle, never the chevron the workspace headers fold
                with: on a phone both controls share the 8–24 column, so the
                glyph is what tells them apart. */}
            {expanded.has(item.foldId) ? (
              <Minus size={16} color={colors.inkSoft} />
            ) : (
              <Plus size={16} color={colors.inkSoft} />
            )}
          </Pressable>
        ) : null}
        <Pressable
          accessibilityRole="button"
          disabled={!commands || deleting}
          onPress={() => {
            if (selecting) {
              if (selectable) toggleSelection(item.key);
              return;
            }
            // A merged row from *another* desktop belongs to a machine this
            // connection is not talking to: the phone holds one active
            // connection, so selecting it here would name a conversation the
            // active desktop has never heard of. The row is opened on the
            // machine that owns it — the same switch the device picker makes.
            // A row of the active desktop (or an unmerged list, where no row
            // carries a source) is the plain selection it always was.
            if (foreign) {
              void remote.openSessionOnDesktop(item.desktopId!, session.sessionId)
                .catch(() => showToast(t("sessions.openOnDesktopFailed", {
                  name: hostName(item.desktopId),
                })));
              return;
            }
            void remote.selectSession(session.sessionId);
          }}
          onLongPress={() => {
            if (!selecting) onMenu(session, item.desktopId ?? null);
          }}
          onPressIn={() => setPressedSessionId(session.sessionId)}
          onPressOut={() =>
            setPressedSessionId(current => (current === session.sessionId ? null : current))
          }
          style={styles.sessionBody}
        >
          <Text numberOfLines={1} ellipsizeMode="tail" style={[styles.title, unread && styles.unreadTitle]}>
            {session.title || t("sessions.unnamed")}
          </Text>
          {/* In the merged list a row carries its machine's glyph — but only
              when more than one machine is in the list, and only when no
              heading above it already says which machine it is: with a single
              desktop the selector above already names it, and under a workspace
              group the heading carries the same glyph, so the row would repeat
              it on every line of that group's list. */}
          {showSource && !item.groupHasSource && item.desktopId && icons?.get(item.desktopId) ? (
            <Text accessibilityLabel={item.desktopId} style={styles.sourceIcon}>
              {icons.get(item.desktopId)}
            </Text>
          ) : null}
          {session.pinned && <Pin size={13} color={colors.accent} />}
          {running ? (
            <ActivityIndicator size={14} color={colors.accent} />
          ) : status === "waiting_approval" ? (
            <CircleAlert size={16} color={colors.warning} />
          ) : unread && (status === "completed" || status === "failed") ? (
            <View
              style={[
                styles.dot,
                { backgroundColor: status === "failed" ? colors.danger : colors.success },
              ]}
            />
          ) : null}
        </Pressable>
        {!selecting && (
          <Pressable
            accessibilityRole="button"
            accessibilityLabel={t("sessions.actions", {
              title: session.title || t("sessions.unnamed"),
            })}
            accessibilityState={{ disabled: deleting || !commands }}
            disabled={deleting || !commands}
            onPress={() => onMenu(session, item.desktopId ?? null)}
            style={[styles.iconButton, (deleting || !commands) && styles.disabled]}
          >
            <MoreHorizontal size={18} color={colors.inkMuted} />
          </Pressable>
        )}
      </View>
    );
  };

  return (
    <View style={styles.container}>
      {Alert.dialog}
      <RenameModal
        generationKey={renamingWorkspace?.workspace.id}
        onClose={() => setRenamingWorkspace(null)}
        renameOpen={renamingWorkspace !== null}
        renameValue={workspaceName}
        setRenameValue={setWorkspaceName}
        submitRename={submitWorkspaceRename}
        t={t}
        titleKey="sessions.renameWorkspace"
      />
      <ActionMenu
        title={menuWorkspace?.workspace.name || t("sessions.workspace")}
        visible={menuWorkspace !== null}
        onClose={() => setMenuWorkspace(null)}
        actions={menuWorkspace ? [
          // Every group's sheet names the machine these commands are addressed
          // to, the one the phone is reading included. They are all that
          // machine's commands — the workspace delete above all, since it is the
          // one action a reader cannot undo — and a merged list is one list of
          // several machines, so which group lives where is the sheet's own
          // question to answer. Naming only a peer's group left the *shape* of a
          // sheet depending on which machine the phone happened to be reading:
          // the same group gained and lost this line as the reader moved.
          ...(menuWorkspace.desktopId
            ? [{
              label: t("sessions.actionsOnDesktop", {
                name: hostName(menuWorkspace.desktopId),
              }),
              heading: true,
            }]
            : []),
          // Every group offers it, wherever its workspace lives: the sheet the
          // reader opened should hold the same items for a peer's group as for
          // this machine's, and the workspace already says which machine the
          // conversation will run on.
          {
            label: t("sessions.new"),
            icon: <Plus size={18} color={colors.accent} />,
            disabled: !menuWorkspaceCommands || deleting,
            // The draft, the machine it is built on and both failure messages
            // are the helper's: one place decides what creating here takes.
            onPress: () => void startConversationInWorkspace(menuWorkspace),
          },
          ...(menuWorkspaceCommands?.capabilities.has("workspace_pinning_v1") ? [{
            // A workspace group is an ordering shortcut like a pinned
            // conversation: pinned groups sit under the pinned conversations
            // and above the unpinned groups (see catalogRows).
            label: t(menuWorkspace.workspace.pinned ? "sessions.unpin" : "sessions.pin"),
            icon: <Pin size={18} color={colors.inkSoft} />,
            disabled: !menuWorkspaceCommands || deleting,
            onPress: () => {
              void menuWorkspaceCommands
                ?.setWorkspacePinned(
                  menuWorkspace.workspaceId,
                  !menuWorkspace.workspace.pinned,
                )
                .catch(() => Alert.alert(t("common.error")));
            },
          }] : []),
          {
            label: t("sessions.renameWorkspace"),
            icon: <Pencil size={18} color={colors.inkSoft} />,
            disabled: !menuWorkspaceCommands || deleting,
            onPress: () => {
              // Prefill with the name in force, so a small correction is an
              // edit rather than a retype.
              setWorkspaceName(menuWorkspace.workspace.name || "");
              setRenamingWorkspace(menuWorkspace);
            },
          },
          {
            label: t("sessions.selectWorkspaceSessions"),
            icon: <ListChecks size={18} color={colors.inkSoft} />,
            disabled: deleting || sessionsInWorkspace(menuWorkspace).length === 0,
            onPress: () => selectWorkspaceSessions(menuWorkspace),
          },
          {
            label: t("sessions.deleteWorkspace"),
            icon: <Trash2 size={18} color={colors.danger} />,
            destructive: true,
            disabled: !menuWorkspaceCommands || deleting,
            onPress: () => confirmDeleteWorkspace(menuWorkspace),
          },
        ] : []}
      />
      <View testID="session-toolbar" style={styles.tools}>
        {selecting ? (
          <View style={styles.selectionBar}>
            <Pressable
              accessibilityRole="button"
              accessibilityLabel={t(allSelected ? "sessions.deselectVisible" : "sessions.selectVisible")}
              disabled={deleting}
              onPress={() =>
                setSelected(current => {
                  const next = new Set(current);
                  for (const row of selectableRows) {
                    if (allSelected) next.delete(row.key);
                    else next.add(row.key);
                  }
                  return next;
                })
              }
              style={styles.selectAll}
            >
              <CheckCheck size={18} color={colors.accent} />
            </Pressable>
            <Text numberOfLines={1} style={styles.selectionCount}>
              {t("sessions.selectedCount", { count: targets.length })}
            </Text>
            <Pressable
              accessibilityRole="button"
              accessibilityLabel={t("sessions.deleteSelected")}
              disabled={deleting || !canDeleteSelected}
              onPress={deleteSelected}
              style={[
                styles.iconButton,
                (deleting || !canDeleteSelected) && styles.disabled,
              ]}
            >
              {deleting ? (
                <ActivityIndicator size={16} color={colors.danger} />
              ) : (
                <Trash2 size={18} color={colors.danger} />
              )}
            </Pressable>
            <Pressable
              accessibilityRole="button"
              accessibilityLabel={t("chat.cancel")}
              disabled={deleting}
              onPress={() => {
                setSelecting(false);
                setSelected(new Set());
              }}
              style={[styles.iconButton, deleting && styles.disabled]}
            >
              <X size={19} color={colors.accent} />
            </Pressable>
          </View>
        ) : searching ? (
          <>
            <View style={styles.search}>
              <Search size={16} color={colors.inkMuted} />
              <TextInput
                accessibilityLabel={t("sessions.search")}
                placeholder={t("sessions.search")}
                placeholderTextColor={colors.inkMuted}
                value={query}
                onChangeText={setQuery}
                autoFocus
                returnKeyType="search"
                autoCapitalize="none"
                autoCorrect={false}
                style={styles.searchInput}
              />
              {!!query && (
                <Pressable
                  accessibilityRole="button"
                  accessibilityLabel={t("sessions.clearSearch")}
                  onPress={() => setQuery("")}
                  style={styles.iconButton}
                >
                  <X size={16} color={colors.inkMuted} />
                </Pressable>
              )}
            </View>
            <Pressable
              accessibilityRole="button"
              accessibilityLabel={t("chat.cancel")}
              onPress={() => {
                setSearching(false);
                setQuery("");
              }}
              style={styles.cancelSearch}
            >
              <Text style={styles.actionText}>{t("chat.cancel")}</Text>
            </Pressable>
          </>
        ) : (
          <>
            <View style={styles.tabs}>
              {TABS.map(value => (
                <Pressable
                  key={value}
                  accessibilityRole="tab"
                  accessibilityLabel={t(value === "workspace" ? "sessions.workspace" : "sessions.conversations")}
                  accessibilityState={{ selected: tab === value }}
                  onPress={() => onTabChange(value)}
                  style={[styles.tab, tab === value && styles.tabActive]}
                >
                  <Text
                    adjustsFontSizeToFit
                    minimumFontScale={0.85}
                    numberOfLines={1}
                    style={[styles.tabText, tab === value && styles.tabTextActive]}
                  >
                    {t(value === "workspace" ? "sessions.workspace" : "sessions.conversations")}
                  </Text>
                </Pressable>
              ))}
            </View>
            <View style={styles.toolActions}>
              <Pressable
                accessibilityRole="button"
                accessibilityLabel={t("sessions.search")}
                onPress={() => setSearching(true)}
                style={styles.manage}
              >
                <Search size={19} color={colors.inkSoft} />
              </Pressable>
              <Pressable
                accessibilityRole="button"
                accessibilityLabel={t("sessions.select")}
                accessibilityState={{ disabled: deleting || selectableRows.length === 0 }}
                disabled={deleting || selectableRows.length === 0}
                onPress={() => {
                  setSelecting(true);
                  setSelected(new Set());
                }}
                style={[styles.manage, (deleting || selectableRows.length === 0) && styles.disabled]}
              >
                <ListChecks size={19} color={colors.inkSoft} />
              </Pressable>
            </View>
          </>
        )}
      </View>
      <Animated.View
        testID="session-list-page"
        onLayout={event => swipe.measure(event.nativeEvent.layout.width)}
        style={[styles.page, { transform: [{ translateX: swipe.translateX }] }]}
        {...swipe.panHandlers}
      >
        {merged !== undefined
          ? <View />
          : null}
        <FlatList
          key={query.trim() ? `${listKey}:search` : listKey}
          data={rows}
          renderItem={renderRow}
          keyExtractor={row => row.key}
          ref={listRef}
          contentOffset={initialOffset}
          onLayout={onLayout}
          onContentSizeChange={onContentSizeChange}
          onScrollBeginDrag={onScrollBeginDrag}
          onScroll={onScroll}
          scrollEventThrottle={16}
          keyboardShouldPersistTaps="handled"
          keyboardDismissMode="on-drag"
          contentContainerStyle={rows.length ? styles.list : styles.empty}
          ListEmptyComponent={
            query.trim() ? (
              <Text style={styles.noResults}>{t("sessions.noResults")}</Text>
            ) : (
              <>{empty}</>
            )
          }
        />
      </Animated.View>
    </View>
  );
}

const styles = StyleSheet.create({
  container: { flex: 1 },
  // The swipeable page: the list keeps its own layout inside it, and the
  // transform that slides it is the only thing this view adds.
  page: { flex: 1 },
  tools: {
    minHeight: layout.touchTarget + spacing.xs * 2 + spacing.md,
    flexDirection: "row",
    alignItems: "center",
    gap: spacing.sm,
    paddingHorizontal: layout.gutter,
    paddingBottom: spacing.md,
  },
  tabs: {
    width: "65%",
    maxWidth: 240,
    flexShrink: 1,
    minWidth: 0,
    flexDirection: "row",
    padding: spacing.xs,
    borderRadius: radius.md,
    backgroundColor: colors.surfaceSubtle,
  },
  tab: {
    flex: 1,
    minWidth: 0,
    minHeight: layout.touchTarget,
    justifyContent: "center",
    paddingHorizontal: spacing.xs,
    paddingVertical: spacing.sm,
    borderRadius: radius.sm,
  },
  tabActive: { backgroundColor: colors.surface },
  tabText: { textAlign: "center", color: colors.inkSoft, fontSize: 14, fontWeight: "600" },
  tabTextActive: { color: colors.accent, fontWeight: "700" },
  toolActions: {
    marginLeft: "auto",
    flexDirection: "row",
    gap: spacing.xs,
  },
  cancelSearch: { minWidth: 44, minHeight: 44, alignItems: "center", justifyContent: "center" },
  search: {
    flex: 1,
    minWidth: 0,
    flexDirection: "row",
    alignItems: "center",
    paddingLeft: spacing.md,
    borderRadius: radius.lg,
    backgroundColor: colors.surfaceSubtle,
  },
  searchInput: {
    flex: 1,
    minWidth: 0,
    minHeight: 44,
    paddingHorizontal: spacing.sm,
    paddingVertical: spacing.sm,
    fontSize: 14,
    color: colors.ink,
  },
  manage: {
    width: 44,
    height: 44,
    alignItems: "center",
    justifyContent: "center",
    borderRadius: radius.lg,
    borderWidth: 1,
    borderColor: colors.lineSoft,
  },
  list: { paddingHorizontal: layout.gutter, paddingTop: spacing.xs, paddingBottom: 96 },
  empty: {
    flexGrow: 1,
    alignItems: "center",
    justifyContent: "center",
    paddingHorizontal: layout.gutter,
    paddingVertical: spacing.xl,
    paddingBottom: 96,
  },
  noResults: { color: colors.inkMuted, fontSize: 14 },
  workspace: {
    minHeight: 44,
    flexDirection: "row",
    alignItems: "center",
    paddingRight: spacing.xs,
    backgroundColor: colors.canvas,
    borderRadius: radius.md,
    marginTop: spacing.sm,
    marginBottom: spacing.xs,
  },
  workspaceBody: {
    flex: 1,
    minWidth: 0,
    minHeight: 44,
    flexDirection: "row",
    alignItems: "center",
    // 4px (not 8): with the 16px folder icon this puts the workspace name on the
    // same column as the group's first-level titles, the way the desktop rail's
    // group header does.
    gap: TOGGLE_GAP,
    paddingLeft: spacing.sm,
    borderRadius: radius.md,
  },
  workspaceName: { flex: 1, color: colors.inkSoft, fontSize: 13, fontWeight: "700" },
  count: { color: colors.inkMuted, fontSize: 12, fontVariant: ["tabular-nums"] },
  row: { minHeight: layout.touchTarget, marginBottom: spacing.xs, flexDirection: "row", alignItems: "center", borderRadius: radius.md },
  rowPressed: { backgroundColor: colors.surfaceSubtle },
  iconButton: { width: 44, minHeight: 44, alignItems: "center", justifyContent: "center" },
  // 16px wide (not 44) so the title column matches the desktop rail; the 44x44
  // touch target comes from the Pressable's hitSlop instead.
  expander: {
    width: TOGGLE_WIDTH,
    minHeight: layout.touchTarget,
    marginRight: TOGGLE_GAP,
    alignItems: "center",
    justifyContent: "center",
  },
  sessionBody: {
    flex: 1,
    minWidth: 0,
    minHeight: layout.touchTarget,
    paddingVertical: spacing.sm,
    flexDirection: "row",
    alignItems: "center",
    gap: spacing.sm,
    borderRadius: radius.md,
  },
  title: { flex: 1, color: colors.ink, fontSize: 15, lineHeight: 22 },
  unreadTitle: { fontWeight: "600" },
  pressed: { backgroundColor: colors.surfaceSubtle },
  selected: { backgroundColor: colors.accentSoft },
  checkbox: {
    width: 18,
    height: 18,
    borderRadius: radius.sm,
    borderWidth: 1,
    borderColor: colors.line,
    alignItems: "center",
    justifyContent: "center",
  },
  checkboxChecked: { backgroundColor: colors.accent, borderColor: colors.accent },
  dot: { width: 7, height: 7, borderRadius: radius.pill },
  sourceIcon: { fontSize: 13, marginLeft: spacing.xs },
  selectionBar: {
    flex: 1,
    minWidth: 0,
    minHeight: 52,
    flexDirection: "row",
    alignItems: "center",
    gap: spacing.xs,
    paddingHorizontal: spacing.xs,
    borderRadius: radius.md,
    backgroundColor: colors.accentSoft,
    borderBottomWidth: StyleSheet.hairlineWidth,
    borderBottomColor: colors.lineSoft,
  },
  selectAll: { width: 44, minHeight: 44, alignItems: "center", justifyContent: "center" },
  selectionCount: { flex: 1, minWidth: 0, color: colors.inkSoft, fontSize: 13, fontVariant: ["tabular-nums"] },
  actionText: { flexShrink: 1, color: colors.accent, fontSize: 13, fontWeight: "600" },
  disabled: { opacity: 0.4 },
});
