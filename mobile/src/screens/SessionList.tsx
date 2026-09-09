import {
  Check,
  CheckCheck,
  ChevronDown,
  ChevronRight,
  CircleAlert,
  Folder,
  ListChecks,
  MoreHorizontal,
  Pin,
  Search,
  Trash2,
  X,
} from "lucide-react-native";
import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  ActivityIndicator,
  Alert,
  BackHandler,
  FlatList,
  Pressable,
  StyleSheet,
  Text,
  TextInput,
  View,
} from "react-native";
import type { ReactNode } from "react";
import { useRemote } from "../remote/RemoteContext";
import { effectiveRunStatus } from "../remote/sessionStatus";
import type { RemoteSession } from "../remote/types";
import { colors, radius, spacing } from "../theme/tokens";
import { catalogRows, type CatalogRow } from "./sessionTree";

// Keep navigation state when the screen unmounts to open a conversation.
let savedCollapsed = new Set<string>();
let savedExpanded = new Set<string>();
const offsets = { chat: 0, workspace: 0 };

export function SessionList({
  tab,
  empty,
  onMenu,
}: {
  tab: "chat" | "workspace";
  empty: ReactNode;
  onMenu: (session: RemoteSession) => void;
}) {
  const remote = useRemote();
  const { t } = useTranslation();
  const [query, setQuery] = useState("");
  const [collapsed, setCollapsed] = useState(savedCollapsed);
  const [expanded, setExpanded] = useState(savedExpanded);
  useEffect(() => {
    savedCollapsed = collapsed;
  }, [collapsed]);
  useEffect(() => {
    savedExpanded = expanded;
  }, [expanded]);
  const [selecting, setSelecting] = useState(false);
  const [selected, setSelected] = useState(new Set<string>());
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
    if (!selecting) return;
    const subscription = BackHandler.addEventListener("hardwareBackPress", () => {
      if (!deletingRef.current) {
        setSelecting(false);
        setSelected(new Set());
      }
      return true;
    });
    return () => subscription.remove();
  }, [selecting]);

  const toggleSelection = (id: string) =>
    setSelected(current => {
      const next = new Set(current);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  const toggleFold = (id: string, workspace: boolean) => {
    const next = new Set(workspace ? collapsed : expanded);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    if (workspace) {
      setCollapsed(next);
    } else {
      setExpanded(next);
    }
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
  const renderRow = ({ item }: { item: CatalogRow }) => {
    if (item.kind === "workspace") {
      const isCollapsed = collapsed.has(item.workspace.id) && !query.trim();
      return (
        <Pressable
          accessibilityRole="button"
          accessibilityLabel={item.workspace.name || t("sessions.workspace")}
          accessibilityState={{ expanded: !isCollapsed }}
          onPress={() => toggleFold(item.workspace.id, true)}
          style={({ pressed }) => [styles.workspace, pressed && styles.pressed]}
        >
          {isCollapsed ? (
            <ChevronRight size={16} color={colors.inkSoft} />
          ) : (
            <ChevronDown size={16} color={colors.inkSoft} />
          )}
          <Folder size={17} color={colors.accent} />
          <Text numberOfLines={1} style={styles.workspaceName}>
            {item.workspace.name || t("sessions.workspace")}
          </Text>
          <Text style={styles.count}>{item.count}</Text>
        </Pressable>
      );
    }
    const session = item.session;
    const checked = selected.has(session.sessionId);
    const status = effectiveRunStatus(session.status, session.streaming);
    const running = status === "running" || status === "queued";
    const unread = remote.unreadSessions.has(session.sessionId);
    return (
      <View style={[styles.row, { marginLeft: item.depth * 16 }, checked && styles.selected]}>
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
            style={styles.iconButton}
          >
            {expanded.has(session.sessionId) ? (
              <ChevronDown size={16} color={colors.inkSoft} />
            ) : (
              <ChevronRight size={16} color={colors.inkSoft} />
            )}
          </Pressable>
        ) : !selecting ? (
          <View style={styles.leafInset} />
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
          style={({ pressed }) => [styles.sessionBody, pressed && styles.pressed]}
        >
          <Text numberOfLines={1} style={[styles.title, unread && styles.unreadTitle]}>
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
            disabled={!remote.desktopOnline}
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
      <View style={styles.tools}>
        <View style={styles.search}>
          <Search size={16} color={colors.inkMuted} />
          <TextInput
            accessibilityLabel={t("sessions.search")}
            placeholder={t("sessions.search")}
            placeholderTextColor={colors.inkMuted}
            value={query}
            onChangeText={setQuery}
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
          accessibilityLabel={t(selecting ? "chat.cancel" : "sessions.select")}
          accessibilityState={{ selected: selecting, disabled: deleting || !remote.desktopOnline }}
          disabled={deleting || !remote.desktopOnline}
          onPress={() => {
            setSelecting(!selecting);
            setSelected(new Set());
          }}
          style={[styles.manage, selecting && styles.selected]}
        >
          {selecting ? (
            <X size={19} color={colors.accent} />
          ) : (
            <ListChecks size={19} color={colors.inkSoft} />
          )}
        </Pressable>
      </View>
      {selecting && (
        <View style={styles.selectionBar}>
          <Pressable
            accessibilityRole="button"
            accessibilityLabel={t(
              allSelected ? "sessions.deselectVisible" : "sessions.selectVisible",
            )}
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
            <CheckCheck size={16} color={colors.accent} />
            <Text style={styles.actionText}>
              {t(allSelected ? "sessions.deselectVisible" : "sessions.selectVisible")}
            </Text>
          </Pressable>
          <Text style={styles.count}>{t("sessions.selectedCount", { count: targets.length })}</Text>
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
        </View>
      )}
      <FlatList
        key={query.trim() ? "search" : tab}
        data={rows}
        renderItem={renderRow}
        keyExtractor={row => row.key}
        contentOffset={{ x: 0, y: query.trim() ? 0 : offsets[tab] }}
        onScroll={event => {
          if (!query.trim()) offsets[tab] = event.nativeEvent.contentOffset.y;
        }}
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
    flexDirection: "row",
    gap: spacing.sm,
    paddingHorizontal: spacing.md,
    paddingBottom: spacing.sm,
  },
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
  list: { paddingHorizontal: spacing.sm, paddingBottom: 84 },
  empty: {
    flexGrow: 1,
    alignItems: "center",
    justifyContent: "center",
    paddingHorizontal: spacing.md,
  },
  noResults: { color: colors.inkMuted, fontSize: 14 },
  workspace: {
    minHeight: 44,
    flexDirection: "row",
    alignItems: "center",
    gap: spacing.sm,
    paddingHorizontal: spacing.sm,
    backgroundColor: colors.canvas,
    borderRadius: radius.md,
    marginTop: spacing.xs,
    marginBottom: 2,
  },
  workspaceName: { flex: 1, color: colors.inkSoft, fontSize: 13, fontWeight: "700" },
  count: { color: colors.inkMuted, fontSize: 12, fontVariant: ["tabular-nums"] },
  row: { minHeight: 44, flexDirection: "row", alignItems: "center", borderRadius: radius.md },
  leafInset: { width: 12 },
  iconButton: { width: 44, minHeight: 44, alignItems: "center", justifyContent: "center" },
  sessionBody: {
    flex: 1,
    minWidth: 0,
    minHeight: 44,
    flexDirection: "row",
    alignItems: "center",
    gap: spacing.sm,
    borderRadius: radius.md,
  },
  title: { flex: 1, color: colors.ink, fontSize: 14, lineHeight: 20 },
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
    flexDirection: "row",
    alignItems: "center",
    gap: spacing.sm,
    paddingLeft: spacing.md,
    paddingRight: spacing.xs,
    borderBottomWidth: StyleSheet.hairlineWidth,
    borderBottomColor: colors.lineSoft,
  },
  selectAll: {
    flex: 1,
    minHeight: 44,
    flexDirection: "row",
    alignItems: "center",
    gap: spacing.sm,
  },
  actionText: { color: colors.accent, fontSize: 13, fontWeight: "600" },
  disabled: { opacity: 0.4 },
});
