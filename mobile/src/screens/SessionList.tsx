import {
  Archive,
  ArchiveRestore,
  ArrowLeft,
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
import { useEffect, useImperativeHandle, useMemo, useRef, useState, type ReactNode, type Ref } from "react";
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
import { ActionMenu } from "../components/ActionMenu";
import { useAppDialog } from "../components/useAppDialog";
import { RenameModal } from "../features/chat/components/RenameModal";
import { showToast } from "../features/chat/utils";
import { PeerIconGlyph } from "../remote/PeerIconGlyph";
import { useRemoteControls as useRemote } from "../remote/RemoteContext";
import type { DesktopCommands } from "../remote/desktopCommands";
import { effectiveRunStatus } from "../remote/sessionStatus";
import type { RemoteSession } from "../remote/types";
import { layout, radius, spacing } from "../theme/tokens";
import { createThemedStyles, useTheme } from "../theme/ThemeContext";
import { formatRelativeTime } from "./relativeTime";
import { RECENT_PAGE_SIZE, recentRows } from "./recentRows";
import type { MergedSessionRow } from "../remote/mergeSessions";
import { activeWorkspaces, ARCHIVED_PAGE_SIZE, archivedCatalogRows, isArchivedSession, type ArchivedCatalog } from "./archived";
import {
  archivedRows,
  catalogRows,
  filterArchivedRows,
  mergedRows as sessionTreeMergedRows,
  pageArchivedRows,
  type CatalogRow,
} from "./sessionTree";
import { listTargets, selectableTargets, type SessionTarget } from "./sessionSelection";
import { swipeTarget, TAB_LABEL, TABS, type Tab } from "./sessionTabs";
import { useCollapsedWorkspaces } from "./useCollapsedWorkspaces";
import { useSessionListScroll } from "./useSessionListScroll";
import { useTabSwipe } from "./useTabSwipe";

// Keep navigation state when the screen unmounts to open a conversation.
// Workspace folds are persisted separately (they survive a restart too).
let savedExpanded = new Set<string>();

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
 * The list's secondary tools, reached from the header rather than from this
 * row: the pages' tabs get the row's whole width.
 */
export type SessionListHandle = {
  /** Open the search field, which replaces the tab row while the reader types. */
  startSearch(): void;
  /**
   * Enter batch selection.
   *
   * A no-op when a batch could hold nothing — the header's menu item is
   * disabled in that state, and this is the same judgement made once more at
   * the point of acting.
   */
  startSelecting(): void;
};

/**
 * A row with the machine it came from attached.
 *
 * The single-machine tree is built from the connected desktop's own catalogue,
 * where a source would be the same fact on every line — so it carries none, and
 * that is the only list on the phone whose rows come from nowhere. The
 * connection the list is drawing is that machine, so it is stamped here, at the
 * one place that knows it, rather than threaded through the tree builder for a
 * fact the tree does not use.
 */
function withMachine(row: CatalogRow, desktopId: string): CatalogRow {
  if (row.kind === "section" || row.desktopId) return row;
  return { ...row, desktopId };
}

/** A batch's rows, grouped by the machine that will delete them. */
function groupByMachine(targets: SessionTarget[]): Map<string | null, SessionTarget[]> {
  const byMachine = new Map<string | null, SessionTarget[]>();
  for (const target of targets) {
    const group = byMachine.get(target.desktopId);
    if (group) group.push(target);
    else byMachine.set(target.desktopId, [target]);
  }
  return byMachine;
}

/**
 * The footer under a paged list.
 *
 * One control for both windows — the recency page's and the archived shelf's —
 * because they say the same thing and a reader should not have to learn the
 * difference: how much is held back, and the way to ask for it.
 */
function ShowMore({ count, onPress }: { count: number; onPress(): void }) {
  const styles = useStyles();
  const { t } = useTranslation();
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={t("sessions.showMore")}
      onPress={onPress}
      style={({ pressed }) => [styles.showMore, pressed && styles.pressed]}
    >
      <Text style={styles.showMoreLabel}>
        {t("sessions.showMoreCount", { count })}
      </Text>
    </Pressable>
  );
}

export function SessionList({
  tab,
  active = true,
  archived = false,
  archivedCatalogs,
  onExitArchived,
  onTabChange,
  empty,
  merged,
  icons,
  activeDesktopId = null,
  commandsFor,
  onMenu,
  ref,
}: {
  tab: Tab;
  active?: boolean;
  /**
   * Whether the list is showing the archived shelf in place of its pages.
   *
   * A mode rather than a second list: the archived view draws the same rows, in
   * the same rows' shape, from the same catalogue — what changes is which half
   * of it, and that a row's menu carries the restore instead of the shelf
   * action. Shelving itself is offered from the active rows, gated on the
   * machine's `archive_v1` capability.
   */
  archived?: boolean;
  /**
   * Every machine the list is showing, for the archived shelf.
   *
   * The shelf is the one view that has to hold all of them: a conversation
   * shelved on a pairing that is not the open connection used to be missing
   * from the one page meant to list it. When absent the shelf falls back to the
   * connected desktop's own catalogue, which is the same thing for a phone with
   * one pairing.
   */
  archivedCatalogs?: ArchivedCatalog[];
  /**
   * Leave the archived shelf for the pages it replaced.
   *
   * The shelf is a destination rather than a fourth page, and a phone has no
   * hardware back for it: without this the only way out was the header's menu —
   * the same control that opened it, one level above the page the reader is
   * looking at.
   */
  onExitArchived(): void;
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
  /**
   * Per-desktop icon id, so every row shows which machine it is on.
   *
   * A machine with no icon of its own is absent from the map (or maps to
   * `undefined`) and draws the shared set's default — the same mark the
   * paired-desktops page and the desktop app give it. The glyph is about
   * *which machine*, and "nobody has named this one yet" is not a state the
   * reader can act on: a row that drew nothing where its neighbours drew a
   * machine read as a row that came from nowhere.
   */
  icons?: Map<string, string | undefined>;
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
  /**
   * The header's way into the toolbar's two secondary modes.
   *
   * The search field and the batch bar live in this component — they hold its
   * rows' state — while the header, which owns the machine and the connection,
   * is the screen's. The screen therefore asks for these two modes through the
   * handle rather than owning them: there is one piece of state, where the rows
   * are.
   */
  ref?: Ref<SessionListHandle>;
}) {
  const styles = useStyles();
  const { colors } = useTheme();
  const remote = useRemote();
  const Alert = useAppDialog(active);
  const { t, i18n } = useTranslation();
  const [query, setQuery] = useState("");
  const [searching, setSearching] = useState(false);
  const listKey = `${remote.credentials?.pairId ?? ""}:${tab}${archived ? ":archived" : ""}`;
  const {
    listRef,
    initialOffset,
    maintainVisibleContentPosition,
    onLayout,
    onContentSizeChange,
    onScrollBeginDrag,
    onScroll,
  } = useSessionListScroll<CatalogRow>(query.trim() ? null : listKey);
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
   * What the recency page reads its ages against.
   *
   * A stamp is not what a reader wants from this page — "3 min ago" is — and an
   * age rendered once would still say "3 min ago" an hour later. The page asks
   * again once a minute while it is the page on screen; a conversation that
   * moves in the meantime is already redrawn, since its stamp changed.
   */
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!active || tab !== "recent") return;
    // The age the page opens with is this list's mounting time, and the list is
    // remounted per page (the screen keys it by tab), so entering the page is
    // what stamps the clock; the timer only keeps it moving while it is read.
    const timer = setInterval(() => setNow(Date.now()), 60_000);
    return () => clearInterval(timer);
  }, [active, tab]);
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
   * The shelved row being brought back, by row key.
   *
   * One at a time, and held across the await: restoring is a write to the
   * machine, and two taps on one row would send it twice. The key rather than a
   * boolean because the shelf can hold several machines' rows and only the row
   * that was tapped may draw the spinner.
   */
  const [restoringKey, setRestoringKey] = useState<string | null>(null);
  const restoringRef = useRef(false);
  /**
   * The machine that owns a row: the one this phone is talking to for a row of
   * its own, and the row's own source for a row merged in from elsewhere.
   */
  const rowCommands = (row: CatalogRow): DesktopCommands | null =>
    commandsFor("desktopId" in row ? row.desktopId ?? null : null);
  /**
   * The recency page's input: the same conversations either list holds, in the
   * one shape that page sorts by.
   *
   * The merged list already carries each row's machine and stamp. The ordinary
   * single-machine list carries the stamp on the session itself, and its rows
   * all belong to the machine the phone is talking to — the source is filled in
   * so one ordering rule serves both shapes.
   */
  const recentSource = useMemo<MergedSessionRow[]>(() => {
    if (merged !== undefined) return merged.filter(row => !isArchivedSession(row.session));
    return remote.sessions.filter(session => !isArchivedSession(session)).map(session => {
      const lastMessageAt = typeof session.lastMessageAt === "number"
        ? session.lastMessageAt
        : null;
      return {
        key: session.sessionId,
        desktopId: activeDesktopId ?? "",
        session,
        lastMessageAt,
      };
    });
  }, [merged, remote.sessions, activeDesktopId]);
  /**
   * How many finished conversations the recency page draws.
   *
   * A window, opened further by the footer under the rows: a reader with a few
   * hundred conversations should not scroll past all of them to reach the bottom
   * of a page whose point is the last few. It starts over when the page does —
   * coming back to "the recent ones" means the recent ones, not wherever the
   * window was left.
   */
  const [recentLimit, setRecentLimit] = useState(RECENT_PAGE_SIZE);
  const recent = useMemo(
    () => (tab === "recent"
      // The fold set and the active desktop are the other pages': one
      // conversation opens and closes the same on all three, instead of a fold
      // that depends on which page the reader happens to be on.
      ? recentRows(recentSource, query, recentLimit, expanded, activeDesktopId)
      : null),
    [tab, recentSource, query, recentLimit, expanded, activeDesktopId],
  );
  /**
   * What the default pages may draw.
   *
   * Archived conversations and archived workspaces belong to the archived
   * view: the desktop hides both from its own default lists, and a page here
   * that drew them would put one conversation in two different places depending
   * on the device. The rows stay in the catalogue (one fetch serves both views),
   * so this is a filter, not a second read.
   */
  const activeSessions = useMemo(
    () => remote.sessions.filter(session => !isArchivedSession(session)),
    [remote.sessions],
  );
  const listedWorkspaces = useMemo(
    () => activeWorkspaces(remote.workspaces),
    [remote.workspaces],
  );
  const activeMerged = useMemo(
    () => (merged !== undefined ? merged.filter(row => !isArchivedSession(row.session)) : undefined),
    [merged],
  );
  /**
   * The archived shelf, rebuilt from the catalogue under the current fold set.
   *
   * While a search is on the shelves are drawn open: a match hidden inside a
   * folded group is a match the reader cannot see, and the default pages'
   * search overrides their folds for the same reason.
   */
  const archivedSource = useMemo(
    () => (archived
      ? archivedCatalogs && archivedCatalogs.length > 0
        ? archivedCatalogRows(archivedCatalogs, query.trim() ? new Set<string>() : collapsed)
        : archivedRows(
          remote.sessions,
          remote.workspaces,
          () => true,
          query.trim() ? new Set<string>() : collapsed,
        )
      : []),
    [archived, archivedCatalogs, remote.sessions, remote.workspaces, collapsed, query],
  );
  /**
   * How many archived conversations the shelf draws.
   *
   * The recency page's window, for the same reason: a shelf is somewhere to look
   * something up, not somewhere to scroll. It starts over whenever the view does.
   */
  const [archivedLimit, setArchivedLimit] = useState(ARCHIVED_PAGE_SIZE);
  const archivedPage = useMemo(
    () => (archived
      ? pageArchivedRows(filterArchivedRows(archivedSource, query), archivedLimit)
      : null),
    [archived, archivedSource, query, archivedLimit],
  );
  const rows = useMemo(() => {
    // `recent` is this page's own rows, computed above under the same conditions
    // — narrowed on `tab` here so the two tree builders keep their own two-page
    // contract. The archived shelf is neither page: it is drawn from its own
    // catalogue half, whole shelves at a time (see `pageArchivedRows`).
    const built = archivedPage
      ? archivedPage.rows
      : tab === "recent"
      ? (recent ? recent.rows : [])
      : activeMerged !== undefined
        ? sessionTreeMergedRows(activeMerged, query, activeDesktopId, tab, collapsed, expanded)
        : catalogRows(activeSessions, listedWorkspaces, tab, collapsed, expanded, query);
    // Every row the list draws belongs to a machine, and every row says which
    // one it is: the single-machine tree is built from the connected desktop's
    // own catalogue and so carries no source of its own, which would leave it
    // the only list on the phone whose rows came from nowhere. Stamped here
    // rather than in the tree builder because it is the list, not the tree,
    // that knows which desktop this connection is to.
    return activeDesktopId ? built.map(row => withMachine(row, activeDesktopId)) : built;
  }, [
    archivedPage,
    activeSessions,
    listedWorkspaces,
    activeMerged,
    tab,
    collapsed,
    expanded,
    query,
    activeDesktopId,
    recent,
  ]);
  /**
   * Every conversation the list holds, folded or not, under the key a batch
   * uses for it.
   *
   * Shared with the screen that offers the batch's entry point (see
   * `listTargets`): the header must not offer a selection the list would refuse
   * to make, and one function answering for both is what keeps them agreeing.
   */
  const allSessions = useMemo<SessionTarget[]>(
    () => (archived ? [] : listTargets(
      merged !== undefined ? merged.filter(row => !isArchivedSession(row.session)) : undefined,
      remote.sessions.filter(session => !isArchivedSession(session)),
    )),
    [archived, merged, remote.sessions],
  );
  /**
   * Whether a pinned conversation is a shortcut on this page rather than an
   * ordinary row.
   *
   * True on the two tree pages, where a pin lifts the row above the groups (and,
   * on the workspace page, out of its own workspace) — so it keeps no checkbox
   * and a group's "select all" never sweeps it up. False on the recency page,
   * which ignores pinning for its order and only draws the marker: there a
   * pinned conversation is a row like any other, and leaving it unselectable put
   * one row of that list beyond the batch for no reason the reader could see.
   */
  const pinnedAreShortcuts = tab !== "recent";
  /**
   * Whether rows are drawn in the recency page's two-line shape.
   *
   * The shelf borrows the workspace pages' one-line row even when the reader
   * arrived from the recency page: its rows are grouped under shelves, and a
   * two-line row there would put an age under a heading that has no time.
   */
  const recentLayout = tab === "recent" && !archived;
  // What a batch may hold: the rows above, on a machine that can take the
  // command. A machine that cannot answer is left out because a checkbox would
  // promise a delete it would refuse.
  //
  // Computed every render rather than memoized: it reads the machines through
  // `commandsFor`, whose answer can change while that callback's identity stays
  // put — a memo keyed on the callback would keep offering a checkbox after the
  // machine behind it went away.
  const selectableRows: SessionRow[] = archived
    ? []
    : rows.filter(
      (row): row is SessionRow =>
        row.kind === "session"
        && (!pinnedAreShortcuts || !row.session.pinned)
        && commandsFor(row.desktopId ?? null) !== null,
    );
  /**
   * Whether the list holds anything a batch may reach *at all*, folded away or
   * not — which is a different question from the one `selectableRows` answers.
   *
   * That one is about the rows on screen, because "select visible" selects the
   * visible ones. This one decides whether entering a batch is offered at all,
   * and a folded group's conversations are still in the list: they are
   * reachable by expanding the group, and by the group's own sheet. Read off the
   * drawn rows, a list of nothing but folded groups and pinned shortcuts — the
   * shape a list takes after a while, and one no batch may hold — drew nothing
   * selectable at all and switched the entry off on a list of hundreds of
   * conversations; the desktop rail's own "select conversations" is offered
   * throughout.
   */
  const batchTargets = archived
    ? []
    : selectableTargets(allSessions, commandsFor, pinnedAreShortcuts);
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

  /**
   * The header's two entry points, as this component's own modes.
   *
   * Both are exclusive: the tab row is replaced by whichever is active, so
   * entering one leaves the other. Batch selection also drops a search filter —
   * the batch spans the catalogue, so leaving a filter on screen would hide rows
   * whose ticks are already counted.
   */
  useImperativeHandle(ref, () => ({
    startSearch: () => {
      setSelecting(false);
      setSelected(new Set());
      setSearching(true);
    },
    startSelecting: () => {
      if (batchTargets.length === 0) return;
      setSearching(false);
      setQuery("");
      setSelecting(true);
      setSelected(new Set());
    },
  }));

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
    enabled: !searching && !selecting && !archived,
    hasPage: direction => swipeTarget(tab, direction) !== null,
    onSwitch: direction => {
      const next = swipeTarget(tab, direction);
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
  const sessionsInWorkspace = (workspace: WorkspaceRow): SessionTarget[] =>
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
  /** Whether the open menu is a shelved shelf, which flips the shelf item. */
  const menuWorkspaceArchived = menuWorkspace?.workspace.archived === true;
  /** Whether that machine answered the archive commands. */
  const menuWorkspaceCanArchive = menuWorkspaceCommands?.capabilities.has("archive_v1") === true;

  /**
   * Whether a row's machine answered the archive commands.
   *
   * Reading a shelf never needed the flag — the catalogue has carried `archived`
   * for longer — but changing one does, and a host that predates the commands
   * answers "Unsupported command". So the shelf's controls are withheld there
   * rather than offered and then refused, exactly as the menu's items are.
   */
  const canShelve = (commands: DesktopCommands | null): boolean =>
    commands?.capabilities.has("archive_v1") === true;

  /**
   * Bring one shelved conversation back, from the shelf itself.
   *
   * The shelf's whole purpose is this one write, so it is a control of its own
   * on the row rather than an item behind the row's ⋯: a page whose only action
   * is restore should not make the reader open a menu per row to find it. The
   * host's catalogue answers with the row moving out of the shelf — this re-reads
   * nothing, because the push that follows the command is the same one the active
   * lists already render from.
   */
  const restoreSessionRow = (row: SessionRow, commands: DesktopCommands | null) => {
    if (!commands || !row.session.threadId || restoringRef.current) return;
    restoringRef.current = true;
    setRestoringKey(row.key);
    void commands.restoreSession(row.session.sessionId, row.session.threadId)
      .catch(() => Alert.alert(t("common.error"), t("sessions.archiveFailed")))
      .finally(() => {
        restoringRef.current = false;
        setRestoringKey(null);
      });
  };

  /** The same write for a shelved workspace, which brings its conversations too. */
  const restoreWorkspaceRow = (row: WorkspaceRow, commands: DesktopCommands | null) => {
    if (!commands || restoringRef.current) return;
    restoringRef.current = true;
    setRestoringKey(row.key);
    void commands.restoreWorkspace(row.workspaceId)
      .catch(() => Alert.alert(t("common.error"), t("sessions.archiveFailed")))
      .finally(() => {
        restoringRef.current = false;
        setRestoringKey(null);
      });
  };

  const renderRow = ({ item }: { item: CatalogRow }) => {
    /**
     * Whether this row belongs to a machine other than the connected one.
     *
     * A merged row carries its source; the ordinary list's rows do not (every
     * row there is the active desktop's).
     */
    if (item.kind === "section") {
      // A caption, not a control: it says how the page is split and how much is
      // under the split, and takes no touch target of its own.
      return (
        <View
          style={styles.section}
          accessibilityRole="header"
          testID={`recent-section:${item.titleKey}`}
        >
          <Text style={styles.sectionTitle}>{t(item.titleKey)}</Text>
          <Text style={styles.sectionCount}>{item.count}</Text>
        </View>
      );
    }
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
            {item.desktopId ? (
              <View accessibilityLabel={item.desktopId} style={styles.sourceIcon}>
                <PeerIconGlyph id={icons?.get(item.desktopId)} size={13} />
              </View>
            ) : null}
            {item.workspace.pinned && <Pin size={13} color={colors.accent} />}
            <Text style={styles.count}>{item.count}</Text>
          </Pressable>
          {/* A shelved shelf's one action is to come back, so it is a control
              here rather than an item in the sheet: the sheet is where a shelved
              heading's rename and delete still live, not where its restore is
              hidden. Withheld on a machine that predates the commands. */}
          {archived && canShelve(commands) && (
            <Pressable
              accessibilityRole="button"
              accessibilityLabel={t("sessions.restore")}
              accessibilityState={{ disabled: restoringKey !== null }}
              disabled={restoringKey !== null}
              onPress={() => restoreWorkspaceRow(item, commands)}
              style={[styles.iconButton, restoringKey !== null && styles.disabled]}
            >
              {restoringKey === item.key ? (
                <ActivityIndicator size={16} color={colors.accent} />
              ) : (
                <ArchiveRestore size={18} color={colors.accent} />
              )}
            </Pressable>
          )}
          {/* A shelved shelf still has a menu: restoring it is the one action
              the archived view exists to offer, and rename/delete are the same
              commands they are anywhere else. The sheet decides which items the
              machine can take (see its actions below). */}
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
    // A pinned session is a shortcut on the tree pages — no checkbox, no place
    // in a batch — but an ordinary row on the recency page (see
    // `pinnedAreShortcuts`). Nor can a conversation on a machine that cannot
    // answer the delete: a checkbox would promise a removal that machine would
    // refuse.
    const selectable = (!pinnedAreShortcuts || !session.pinned) && commands !== null;
    const status = effectiveRunStatus(session.status, session.streaming);
    const running = status === "running" || status === "queued";
    const unread = remote.unreadSessions.has(session.sessionId);
    /** The row's run indicator: spinning, waiting on the reader, or unread. */
    const statusIndicator = running ? (
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
    ) : null;
    return (
      <View
        style={[styles.row, { paddingLeft: depthInset(item.depth) }, pressedSessionId === session.sessionId && styles.rowPressed]}
        // The recency page's rows are addressed by key in the screenshot
        // scenario, which asserts how many of them the window draws and where
        // the footer under them sits.
        {...(recentLayout ? { testID: `recent-row:${item.section ?? "earlier"}:${item.key}` } : {})}
      >
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
            // The shelf's rows open the same sheet the ⋯ does: it carries
            // rename, delete and — on the machines that answered `archive_v1` —
            // the shelf item, so a long press is not a way into writes the page
            // hides (the visible restore beside it is the one that matters).
            if (!selecting) onMenu(session, item.desktopId ?? null);
          }}
          onPressIn={() => setPressedSessionId(session.sessionId)}
          onPressOut={() =>
            setPressedSessionId(current => (current === session.sessionId ? null : current))
          }
          style={[styles.sessionBody, recentLayout && styles.recentBody]}
        >
          {recentLayout ? (
            <>
              {/* Two lines, because this page's row has more to say than a
                  title: when the conversation last moved, and where it lives.
                  The age sits on the title's line so a column of them reads as
                  one ranking; the workspace goes under it, where it is the
                  quieter fact. */}
              <View style={styles.recentTop}>
                <Text numberOfLines={1} ellipsizeMode="tail" style={[styles.title, unread && styles.unreadTitle]}>
                  {session.title || t("sessions.unnamed")}
                </Text>
                {statusIndicator}
                <Text style={styles.recentTime}>
                  {typeof session.lastMessageAt === "number"
                    ? formatRelativeTime(session.lastMessageAt, now, i18n.language, t)
                    : ""}
                </Text>
              </View>
              <View style={styles.recentBottom}>
                {/* The machine comes first, before the name it belongs to: it
                    opens the under-line at the row's own left edge, so the
                    glyph column lines up with the title above it instead of
                    trailing whatever the name happens to be. */}
                {item.desktopId ? (
                  <View accessibilityLabel={item.desktopId} style={[styles.sourceIcon, styles.leadingSourceIcon]}>
                    <PeerIconGlyph id={icons?.get(item.desktopId)} size={13} />
                  </View>
                ) : null}
                <Text numberOfLines={1} style={styles.recentWhere}>
                  {session.mode === "workspace"
                    ? session.workspaceName || t("sessions.workspace")
                    : t("sessions.chatLabel")}
                </Text>
                {session.pinned && <Pin size={13} color={colors.accent} />}
              </View>
            </>
          ) : (
            <>
          <Text numberOfLines={1} ellipsizeMode="tail" style={[styles.title, unread && styles.unreadTitle]}>
            {session.title || t("sessions.unnamed")}
          </Text>
          {/* Every row carries its machine's glyph — on a phone every
              conversation is on a remote desktop, so a row without one reads as
              a row that came from nowhere, and the selector above names the
              machine for the whole list rather than for the line being read. It
              is the same on a list of one machine, under a workspace heading
              that carries the same mark, and on a row the selector already
              filtered to. A machine the user never gave an icon draws the
              default one. */}
          {item.desktopId ? (
            <View accessibilityLabel={item.desktopId} style={styles.sourceIcon}>
              <PeerIconGlyph id={icons?.get(item.desktopId)} size={13} />
            </View>
          ) : null}
          {session.pinned && <Pin size={13} color={colors.accent} />}
          {statusIndicator}
            </>
          )}
        </Pressable>
        {archived && !selecting && canShelve(commands) && (
          <Pressable
            accessibilityRole="button"
            accessibilityLabel={t("sessions.restore")}
            accessibilityState={{ disabled: restoringKey !== null }}
            disabled={restoringKey !== null}
            onPress={() => restoreSessionRow(item, commands)}
            style={[styles.iconButton, restoringKey !== null && styles.disabled]}
          >
            {restoringKey === item.key ? (
              <ActivityIndicator size={16} color={colors.accent} />
            ) : (
              <ArchiveRestore size={18} color={colors.accent} />
            )}
          </Pressable>
        )}
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
          // conversation will run on. A shelved shelf offers neither this nor the
          // pin: starting work inside one, or pinning one, is a statement about a
          // group the reader cannot see — restoring it is.
          ...(menuWorkspaceArchived
            ? []
            : [{
              label: t("sessions.new"),
              icon: <Plus size={18} color={colors.accent} />,
              disabled: !menuWorkspaceCommands || deleting,
              // The draft, the machine it is built on and both failure messages
              // are the helper's: one place decides what creating here takes.
              onPress: () => void startConversationInWorkspace(menuWorkspace),
            }]),
          ...(menuWorkspaceArchived
            ? []
            : menuWorkspaceCommands?.capabilities.has("workspace_pinning_v1")
              ? [{
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
              }]
              : []),
          // The shelf item flips with the row: shelf a workspace from the active
          // list, restore it from the archived one. Both are the host's own
          // commands, offered only where it declared them.
          ...(menuWorkspaceCanArchive
            ? [{
              label: t(menuWorkspaceArchived ? "sessions.restore" : "sessions.archive"),
              icon: menuWorkspaceArchived
                ? <ArrowLeft size={18} color={colors.inkSoft} />
                : <Archive size={18} color={colors.inkSoft} />,
              disabled: !menuWorkspaceCommands || deleting,
              onPress: () => {
                const commands = menuWorkspaceCommands;
                if (!commands) return;
                setMenuWorkspace(null);
                void (menuWorkspaceArchived
                  ? commands.restoreWorkspace(menuWorkspace.workspaceId)
                  : commands.archiveWorkspace(menuWorkspace.workspaceId))
                  .catch(() => Alert.alert(t("common.error"), t("sessions.archiveFailed")));
              },
            }]
            : []),
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
          ...(menuWorkspaceArchived
            ? []
            : [{
              label: t("sessions.selectWorkspaceSessions"),
              icon: <ListChecks size={18} color={colors.inkSoft} />,
              disabled: deleting || sessionsInWorkspace(menuWorkspace).length === 0,
              onPress: () => selectWorkspaceSessions(menuWorkspace),
            }]),
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
        ) : archived ? (
          // The shelf is not another page of the three: the tab row would invite
          // the reader to swipe between a place they went to and the pages it
          // replaced. So the row carries the shelf's own caption — and the way
          // back, which is a control here rather than a menu item one level above
          // the page: a destination with only the header's ⋯ to leave by is a
          // page the reader is stuck on once they forget how they got there.
          <View style={styles.archivedBar}>
            <Pressable
              accessibilityRole="button"
              accessibilityLabel={t("sessions.archivedBack")}
              onPress={onExitArchived}
              style={({ pressed }) => [styles.archivedBack, pressed && styles.pressed]}
            >
              <ArrowLeft size={18} color={colors.accent} />
              <Text style={styles.archivedBackLabel}>{t("sessions.archivedBack")}</Text>
            </Pressable>
            <Text style={styles.archivedCaptionText} numberOfLines={1}>
              {t("sessions.archived")}
            </Text>
          </View>
        ) : (
          <View style={styles.tabs}>
            {TABS.map(value => (
              <Pressable
                key={value}
                accessibilityRole="tab"
                accessibilityLabel={t(TAB_LABEL[value])}
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
                  {t(TAB_LABEL[value])}
                </Text>
              </Pressable>
            ))}
          </View>
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
          maintainVisibleContentPosition={maintainVisibleContentPosition}
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
          ListFooterComponent={
            // The window's edge, whichever window it is. It says how much is
            // held back rather than opening silently on scroll: both lists are
            // meant to be short, and a reader who wants the rest asks for it.
            //
            // Each list pages only its own rows. The shelf used to fall through
            // to the recency page's window whenever its own was empty, so an
            // empty shelf drew "show 755 more" — the active list's count, under
            // a caption saying nothing was archived, which reads as archived
            // conversations the reader cannot reach.
            archived ? (
              archivedPage && archivedPage.more > 0 ? (
                <ShowMore
                  count={archivedPage.more}
                  onPress={() => setArchivedLimit(current => current + ARCHIVED_PAGE_SIZE)}
                />
              ) : undefined
            ) : recent && recent.more > 0 ? (
              <ShowMore
                count={recent.more}
                onPress={() => setRecentLimit(current => current + RECENT_PAGE_SIZE)}
              />
            ) : undefined
          }
        />
      </Animated.View>
    </View>
  );
}

const useStyles = createThemedStyles(theme => StyleSheet.create({
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
  // The archived shelf's own row: its caption, and the way back to the pages it
  // replaced. Same height as the tab row, so the rows below do not jump when the
  // reader moves between the two.
  archivedBar: {
    flex: 1,
    minWidth: 0,
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    gap: spacing.sm,
    minHeight: layout.touchTarget,
    paddingHorizontal: spacing.xs,
  },
  archivedBack: {
    flexDirection: "row",
    alignItems: "center",
    gap: spacing.xs,
    minHeight: layout.touchTarget,
    paddingRight: spacing.sm,
  },
  archivedBackLabel: { flexShrink: 1, color: theme.colors.accent, fontSize: theme.font(14), fontWeight: "600" },
  archivedCaptionText: {
    flexShrink: 1,
    color: theme.colors.inkSoft,
    fontSize: theme.font(14),
    fontWeight: "600",
  },
  tabs: {
    flex: 1,
    minWidth: 0,
    flexDirection: "row",
    padding: spacing.xs,
    borderRadius: radius.md,
    backgroundColor: theme.colors.surfaceSubtle,
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
  tabActive: { backgroundColor: theme.colors.surface },
  tabText: { textAlign: "center", color: theme.colors.inkSoft, fontSize: theme.font(14), fontWeight: "600" },
  tabTextActive: { color: theme.colors.accent, fontWeight: "700" },
  cancelSearch: { minWidth: 44, minHeight: 44, alignItems: "center", justifyContent: "center" },
  search: {
    flex: 1,
    minWidth: 0,
    flexDirection: "row",
    alignItems: "center",
    paddingLeft: spacing.md,
    borderRadius: radius.lg,
    backgroundColor: theme.colors.surfaceSubtle,
  },
  searchInput: {
    flex: 1,
    minWidth: 0,
    minHeight: 44,
    paddingHorizontal: spacing.sm,
    paddingVertical: spacing.sm,
    fontSize: theme.font(14),
    color: theme.colors.ink,
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
  noResults: { color: theme.colors.inkMuted, fontSize: theme.font(14) },
  // A caption inside the list: the page's two stretches, named once and never
  // sticky (two short captions do not justify a second scrolling structure).
  section: {
    minHeight: 32,
    flexDirection: "row",
    alignItems: "baseline",
    gap: spacing.sm,
    marginTop: spacing.sm,
    paddingHorizontal: spacing.xs,
    paddingBottom: spacing.xs,
  },
  sectionTitle: {
    flexShrink: 1,
    color: theme.colors.inkSoft,
    fontSize: theme.font(12),
    fontWeight: "700",
    textTransform: "uppercase",
    letterSpacing: 0.5,
  },
  sectionCount: {
    color: theme.colors.inkMuted,
    fontSize: theme.font(12),
    fontVariant: ["tabular-nums"],
  },
  workspace: {
    minHeight: 44,
    flexDirection: "row",
    alignItems: "center",
    paddingRight: spacing.xs,
    backgroundColor: theme.colors.canvas,
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
  workspaceName: { flex: 1, color: theme.colors.inkSoft, fontSize: theme.font(13), fontWeight: "700" },
  count: { color: theme.colors.inkMuted, fontSize: theme.font(12), fontVariant: ["tabular-nums"] },
  row: { minHeight: layout.touchTarget, marginBottom: spacing.xs, flexDirection: "row", alignItems: "center", borderRadius: radius.md },
  rowPressed: { backgroundColor: theme.colors.surfaceSubtle },
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
  title: { flex: 1, color: theme.colors.ink, fontSize: theme.font(15), lineHeight: theme.leading(22) },
  // The recency page's window edge: a full-width row under the last drawn
  // conversation, so it reads as "there is more" rather than as a toolbar.
  showMore: {
    minHeight: layout.touchTarget,
    alignItems: "center",
    justifyContent: "center",
    marginTop: spacing.xs,
    borderRadius: radius.md,
    borderWidth: 1,
    borderColor: theme.colors.lineSoft,
    backgroundColor: theme.colors.surfaceSubtle,
  },
  showMoreLabel: { color: theme.colors.accent, fontSize: theme.font(14), fontWeight: "600" },
  // The recency row: title and age on one line, where it lives under them.
  recentBody: { flexDirection: "column", alignItems: "stretch", gap: 2 },
  recentTop: { flexDirection: "row", alignItems: "center", gap: spacing.sm },
  recentTime: {
    color: theme.colors.inkMuted,
    fontSize: theme.font(12),
    fontVariant: ["tabular-nums"],
  },
  recentBottom: { flexDirection: "row", alignItems: "center", gap: spacing.xs },
  recentWhere: { flexShrink: 1, color: theme.colors.inkMuted, fontSize: theme.font(12) },
  unreadTitle: { fontWeight: "600" },
  pressed: { backgroundColor: theme.colors.surfaceSubtle },
  selected: { backgroundColor: theme.colors.accentSoft },
  checkbox: {
    width: 18,
    height: 18,
    borderRadius: radius.sm,
    borderWidth: 1,
    borderColor: theme.colors.line,
    alignItems: "center",
    justifyContent: "center",
  },
  checkboxChecked: { backgroundColor: theme.colors.accent, borderColor: theme.colors.accent },
  dot: { width: 7, height: 7, borderRadius: radius.pill },
  sourceIcon: { marginLeft: spacing.xs },
  // First in its row, where the margin the glyph carried as a follower would
  // read as an indent against the title's edge: the pair keeps its 8px
  // separation (the row's own 4px gap plus this one) on the other side.
  leadingSourceIcon: { marginLeft: 0, marginRight: spacing.xs },
  selectionBar: {
    flex: 1,
    minWidth: 0,
    minHeight: 52,
    flexDirection: "row",
    alignItems: "center",
    gap: spacing.xs,
    paddingHorizontal: spacing.xs,
    borderRadius: radius.md,
    backgroundColor: theme.colors.accentSoft,
    borderBottomWidth: StyleSheet.hairlineWidth,
    borderBottomColor: theme.colors.lineSoft,
  },
  selectAll: { width: 44, minHeight: 44, alignItems: "center", justifyContent: "center" },
  selectionCount: { flex: 1, minWidth: 0, color: theme.colors.inkSoft, fontSize: theme.font(13), fontVariant: ["tabular-nums"] },
  actionText: { flexShrink: 1, color: theme.colors.accent, fontSize: theme.font(13), fontWeight: "600" },
  disabled: { opacity: 0.4 },
}));
