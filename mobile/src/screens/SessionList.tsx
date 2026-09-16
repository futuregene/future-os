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
import { useRemoteControls as useRemote } from "../remote/RemoteContext";
import { effectiveRunStatus } from "../remote/sessionStatus";
import type { RemoteSession } from "../remote/types";
import { colors, layout, radius, spacing } from "../theme/tokens";
import { catalogRows, type CatalogRow } from "./sessionTree";
import { useCollapsedWorkspaces } from "./useCollapsedWorkspaces";
import { useSessionListScroll } from "./useSessionListScroll";

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

export function SessionList({
  tab,
  active = true,
  onTabChange,
  empty,
  onMenu,
}: {
  tab: "chat" | "workspace";
  active?: boolean;
  onTabChange: (tab: "chat" | "workspace") => void;
  empty: ReactNode;
  onMenu: (session: RemoteSession) => void;
}) {
  const remote = useRemote();
  const Alert = useAppDialog(active);
  const { t } = useTranslation();
  const [query, setQuery] = useState("");
  const [searching, setSearching] = useState(false);
  const listKey = `${remote.credentials?.pairId ?? ""}:${tab}`;
  const { listRef, initialOffset, onLayout, onContentSizeChange, onScrollBeginDrag, onScroll } =
    useSessionListScroll<CatalogRow>(query.trim() ? null : listKey);
  const [menuWorkspace, setMenuWorkspace] = useState<(CatalogRow & { kind: "workspace" }) | null>(null);
  const [wasActive, setWasActive] = useState(active);
  if (wasActive !== active) {
    setWasActive(active);
    if (!active) setMenuWorkspace(null);
  }
  const { collapsed, toggleWorkspaceCollapsed } = useCollapsedWorkspaces();
  const [expanded, setExpanded] = useState(savedExpanded);
  useEffect(() => {
    savedExpanded = expanded;
  }, [expanded]);
  const [selecting, setSelecting] = useState(false);
  const [selected, setSelected] = useState(new Set<string>());
  const [pressedSessionId, setPressedSessionId] = useState<string | null>(null);
  const [deleting, setDeleting] = useState(false);
  const deletingRef = useRef(false);
  const rows = useMemo(
    () => catalogRows(remote.sessions, remote.workspaces, tab, collapsed, expanded, query),
    [remote.sessions, remote.workspaces, tab, collapsed, expanded, query],
  );
  const visibleSessions = rows.flatMap(row => (row.kind === "session" ? [row.session] : []));
  const targets = remote.sessions.filter(session => selected.has(session.sessionId));
  const allSelected =
    visibleSessions.length > 0 && visibleSessions.every(session => selected.has(session.sessionId));

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
    if (!targets.length || !remote.desktopOnline || deletingRef.current) return;
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
              // Sequential requests avoid flooding the desktop; keep failures selected for retry.
              for (const session of targets) {
                try {
                  await remote.deleteSession(session.sessionId, session.threadId);
                } catch {
                  failed.add(session.sessionId);
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

  /** Every session in a workspace, flattened — folds must not hide rows from a
   * "select all in this workspace", which is the point of the action. */
  const sessionsInWorkspace = (workspaceId: string): RemoteSession[] =>
    remote.sessions.filter(
      session => session.mode === "workspace" && (session.workspaceId ?? "") === workspaceId,
    );

  const selectWorkspaceSessions = (workspaceId: string) => {
    const ids = sessionsInWorkspace(workspaceId).map(session => session.sessionId);
    if (ids.length === 0) return;
    setSelecting(true);
    setSelected(current => new Set([...current, ...ids]));
  };

  const confirmDeleteWorkspace = (workspace: CatalogRow & { kind: "workspace" }) => {
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
            if (deletingRef.current) return;
            deletingRef.current = true;
            setDeleting(true);
            const removed = new Set(sessionsInWorkspace(workspace.workspace.id).map(s => s.sessionId));
            void remote
              .deleteWorkspace(workspace.workspace.id)
              .then(() => {
                setSelected(current => {
                  const next = new Set(current);
                  for (const id of removed) next.delete(id);
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

  const openWorkspaceMenu = (workspace: CatalogRow & { kind: "workspace" }) => {
    if (!remote.desktopOnline || deleting) return;
    setMenuWorkspace(workspace);
  };

  const renderRow = ({ item }: { item: CatalogRow }) => {
    if (item.kind === "workspace") {
      const isCollapsed = collapsed.has(item.workspace.id) && !query.trim();
      const name = item.workspace.name || t("sessions.workspace");
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
            {item.workspace.pinned && <Pin size={13} color={colors.accent} />}
            <Text style={styles.count}>{item.count}</Text>
          </Pressable>
          <Pressable
            accessibilityRole="button"
            accessibilityLabel={t("sessions.workspaceActions", { title: name })}
            accessibilityState={{ disabled: deleting || !remote.desktopOnline }}
            disabled={deleting || !remote.desktopOnline}
            onPress={() => openWorkspaceMenu(item)}
            style={[styles.iconButton, (!remote.desktopOnline || deleting) && styles.disabled]}
          >
            <MoreHorizontal size={18} color={colors.inkMuted} />
          </Pressable>
        </View>
      );
    }
    const session = item.session;
    const checked = selected.has(session.sessionId);
    const status = effectiveRunStatus(session.status, session.streaming);
    const running = status === "running" || status === "queued";
    const unread = remote.unreadSessions.has(session.sessionId);
    return (
      <View style={[styles.row, { paddingLeft: depthInset(item.depth) }, pressedSessionId === session.sessionId && styles.rowPressed]}>
        {selecting ? (
          <Pressable
            accessibilityRole="checkbox"
            accessibilityLabel={session.title || t("sessions.unnamed")}
            accessibilityState={{ checked, disabled: deleting || !remote.desktopOnline }}
            disabled={deleting || !remote.desktopOnline}
            onPress={() => toggleSelection(session.sessionId)}
            style={styles.iconButton}
          >
            <View style={[styles.checkbox, checked && styles.checkboxChecked]}>
              {checked && <Check size={13} color={colors.surface} />}
            </View>
          </Pressable>
        ) : null}
        {item.hasChildren ? (
          <Pressable
            accessibilityRole="button"
            accessibilityLabel={t(
              expanded.has(session.sessionId)
                ? "sessions.collapseChildren"
                : "sessions.expandChildren",
            )}
            accessibilityState={{ expanded: expanded.has(session.sessionId) }}
            onPress={() => toggleFold(session.sessionId, false)}
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
            {expanded.has(session.sessionId) ? (
              <Minus size={16} color={colors.inkSoft} />
            ) : (
              <Plus size={16} color={colors.inkSoft} />
            )}
          </Pressable>
        ) : null}
        <Pressable
          accessibilityRole="button"
          disabled={!remote.desktopOnline || deleting}
          onPress={() =>
            selecting
              ? toggleSelection(session.sessionId)
              : void remote.selectSession(session.sessionId)
          }
          onLongPress={() => {
            if (!selecting) onMenu(session);
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
            disabled={!remote.desktopOnline || deleting}
            onPress={() => onMenu(session)}
            style={styles.iconButton}
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
      <ActionMenu
        title={menuWorkspace?.workspace.name || t("sessions.workspace")}
        visible={menuWorkspace !== null}
        onClose={() => setMenuWorkspace(null)}
        actions={menuWorkspace ? [
          {
            label: t("sessions.new"),
            icon: <Plus size={18} color={colors.accent} />,
            disabled: !remote.desktopOnline || deleting,
            onPress: () => {
              void remote.newConversation("workspace", menuWorkspace.workspace.id)
                .catch(() => Alert.alert(t("common.error")));
            },
          },
          {
            // A workspace group is an ordering shortcut like a pinned
            // conversation: pinned groups sit under the pinned conversations
            // and above the unpinned groups (see catalogRows).
            label: t(menuWorkspace.workspace.pinned ? "sessions.unpin" : "sessions.pin"),
            icon: <Pin size={18} color={colors.inkSoft} />,
            disabled: !remote.desktopOnline || deleting,
            onPress: () => {
              void remote
                .setWorkspacePinned(
                  menuWorkspace.workspace.id,
                  !menuWorkspace.workspace.pinned,
                )
                .catch(() => Alert.alert(t("common.error")));
            },
          },
          {
            label: t("sessions.selectWorkspaceSessions"),
            icon: <ListChecks size={18} color={colors.inkSoft} />,
            disabled: !remote.desktopOnline || deleting || menuWorkspace.count === 0,
            onPress: () => selectWorkspaceSessions(menuWorkspace.workspace.id),
          },
          {
            label: t("sessions.deleteWorkspace"),
            icon: <Trash2 size={18} color={colors.danger} />,
            destructive: true,
            disabled: !remote.desktopOnline || deleting,
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
              disabled={deleting || !remote.desktopOnline}
              onPress={() =>
                setSelected(current => {
                  const next = new Set(current);
                  for (const session of visibleSessions) {
                    if (allSelected) next.delete(session.sessionId);
                    else next.add(session.sessionId);
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
              disabled={deleting || !targets.length || !remote.desktopOnline}
              onPress={deleteSelected}
              style={[
                styles.iconButton,
                (!targets.length || !remote.desktopOnline) && styles.disabled,
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
              {(["workspace", "chat"] as const).map(value => (
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
                accessibilityState={{ disabled: deleting || !remote.desktopOnline }}
                disabled={deleting || !remote.desktopOnline}
                onPress={() => {
                  setSelecting(true);
                  setSelected(new Set());
                }}
                style={[styles.manage, (deleting || !remote.desktopOnline) && styles.disabled]}
              >
                <ListChecks size={19} color={colors.inkSoft} />
              </Pressable>
            </View>
          </>
        )}
      </View>
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
    </View>
  );
}

const styles = StyleSheet.create({
  container: { flex: 1 },
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
