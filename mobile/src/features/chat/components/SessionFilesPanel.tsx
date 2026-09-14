import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { ArrowUp, ChevronRight, Eye, EyeOff, File, Folder, RefreshCw } from "lucide-react-native";
import { ActivityIndicator, Alert, BackHandler, FlatList, Pressable, StyleSheet, Text, View } from "react-native";
import type { SessionFileListing } from "../../../remote/types";
import { colors, layout, radius, spacing } from "../../../theme/tokens";
import { formatBytes } from "../utils";

/** An in-screen surface, not a native Modal: file previews/download dialogs can
 * present above it on iOS without competing UIKit modal presentations. */
export function SessionFilesPanel({
  online,
  supported,
  isWorkspace,
  listFiles,
  onOpenFile,
  onClose,
}: {
  online: boolean;
  supported: boolean;
  isWorkspace: boolean;
  listFiles: (path?: string) => Promise<SessionFileListing>;
  onOpenFile: (path: string) => Promise<void>;
  onClose: () => void;
}) {
  const { t } = useTranslation();
  const [directories, setDirectories] = useState<string[]>([]);
  const [revision, setRevision] = useState(0);
  const [result, setResult] = useState<{
    request: object;
    listing: SessionFileListing | null;
    failed: boolean;
  } | null>(null);
  const [opening, setOpening] = useState<string | null>(null);
  const [showHidden, setShowHidden] = useState(isWorkspace);
  const path = directories[directories.length - 1] ?? "";

  useEffect(() => {
    // Android's edge-back gesture is delivered as hardwareBackPress, including
    // in APK compatibility runtimes. Only leave the panel at the session root.
    // The top-left header button remains an explicit shortcut back to chat.
    const subscription = BackHandler.addEventListener("hardwareBackPress", () => {
      if (directories.length > 0) setDirectories(value => value.slice(0, -1));
      else onClose();
      return true;
    });
    return () => subscription.remove();
  }, [directories.length, onClose]);

  const request = useMemo(() => ({ listFiles, path, revision, online, supported }),
    [listFiles, path, revision, online, supported]);
  const available = online && supported;
  const settled = result?.request === request;
  const listing = available && settled ? result.listing : null;
  const failed = available && settled && result.failed;
  const loading = available && !settled;

  useEffect(() => {
    let current = true;
    if (request.online && request.supported) {
      void request.listFiles(request.path).then(
        listing => { if (current) setResult({ request, listing, failed: false }); },
        () => { if (current) setResult({ request, listing: null, failed: true }); },
      );
    }
    return () => { current = false; };
  }, [request]);

  const entries = useMemo(
    () => (listing?.entries ?? []).filter(entry => showHidden || !entry.name.startsWith(".")),
    [listing, showHidden],
  );
  const refresh = () => setRevision(value => value + 1);
  const status = !online ? t("files.offline") : !supported ? t("files.updateDesktop")
    : failed ? t("files.loadFailed") : t("files.empty");

  return (
    <View style={styles.panel}>
      <View style={styles.toolbar}>
        <Text style={styles.heading}>{t("files.title")}</Text>
        <Pressable
          accessibilityRole="button"
          accessibilityLabel={showHidden ? t("files.hideHidden") : t("files.showHidden")}
          onPress={() => setShowHidden(value => !value)}
          style={({ pressed }) => [styles.iconButton, pressed && styles.pressed]}
        >
          {showHidden ? <EyeOff size={20} color={colors.inkMuted} /> : <Eye size={20} color={colors.inkMuted} />}
        </Pressable>
        <Pressable
          accessibilityRole="button"
          accessibilityLabel={t("files.refresh")}
          accessibilityState={{ disabled: !available || loading }}
          disabled={!available || loading}
          onPress={refresh}
          style={({ pressed }) => [styles.iconButton, pressed && styles.pressed, (!available || loading) && styles.disabled]}
        >
          <RefreshCw size={20} color={colors.inkMuted} />
        </Pressable>
      </View>
      <Text selectable style={styles.path}>{listing?.path || path || t("files.currentDirectory")}</Text>
      <View style={styles.navigation}>
        <Pressable
          accessibilityRole="button"
          accessibilityLabel={t("files.up")}
          accessibilityState={{ disabled: directories.length === 0 }}
          disabled={directories.length === 0}
          onPress={() => setDirectories(value => value.slice(0, -1))}
          style={({ pressed }) => [styles.navButton, pressed && styles.pressed, directories.length === 0 && styles.disabled]}
        >
          <ArrowUp size={18} color={colors.inkMuted} />
          <Text style={styles.navLabel}>{t("files.up")}</Text>
        </Pressable>
        <Pressable
          accessibilityRole="button"
          accessibilityLabel={t("files.root")}
          disabled={directories.length === 0}
          accessibilityState={{ disabled: directories.length === 0 }}
          onPress={() => setDirectories([])}
          style={({ pressed }) => [styles.navButton, pressed && styles.pressed, directories.length === 0 && styles.disabled]}
        >
          <Text style={styles.navLabel}>{t("files.root")}</Text>
        </Pressable>
      </View>
      {loading ? (
        <View style={styles.empty}>
          <ActivityIndicator color={colors.accent} />
          <Text style={styles.status}>{t("files.loading")}</Text>
        </View>
      ) : (
        <FlatList
          data={entries}
          keyExtractor={entry => entry.path}
          contentContainerStyle={entries.length === 0 ? styles.empty : styles.list}
          ListEmptyComponent={
            <View style={styles.emptyContent}>
              <Text style={styles.status}>{status}</Text>
              {failed && available && (
                <Pressable accessibilityRole="button" onPress={refresh} style={styles.navButton}>
                  <Text style={styles.navLabel}>{t("common.retry")}</Text>
                </Pressable>
              )}
            </View>
          }
          renderItem={({ item }) => (
            <Pressable
              accessibilityRole="button"
              accessibilityLabel={t(item.isDir ? "files.openFolder" : "files.openFile", { name: item.name })}
              accessibilityState={{ disabled: opening !== null, busy: opening === item.path }}
              disabled={opening !== null}
              onPress={() => {
                if (item.isDir) {
                  setDirectories(value => [...value, item.path]);
                } else {
                  setOpening(item.path);
                  // The download controller owns errors and the preview/native
                  // open handoff; keep the directory mounted beneath them.
                  void onOpenFile(item.path)
                    .catch(() => Alert.alert(t("files.title"), t("attachment.downloadFailed")))
                    .finally(() => setOpening(null));
                }
              }}
              style={({ pressed }) => [styles.row, pressed && styles.pressed]}
            >
              {item.isDir ? <Folder color={colors.accent} size={22} /> : <File color={colors.inkMuted} size={22} />}
              <Text numberOfLines={2} style={styles.name}>{item.name}</Text>
              {opening === item.path ? <ActivityIndicator color={colors.accent} size="small" />
                : item.isDir ? <ChevronRight color={colors.inkMuted} size={18} />
                  : <Text style={styles.size}>{formatBytes(item.size)}</Text>}
            </Pressable>
          )}
        />
      )}
    </View>
  );
}

const styles = StyleSheet.create({
  panel: { position: "absolute", top: 0, bottom: 0, left: 0, right: 0, zIndex: 4, backgroundColor: colors.surface },
  toolbar: { flexDirection: "row", alignItems: "center", paddingHorizontal: layout.gutter, paddingTop: spacing.sm },
  heading: { flex: 1, color: colors.inkStrong, fontSize: 18, fontWeight: "700" },
  iconButton: { width: layout.touchTarget, height: layout.touchTarget, alignItems: "center", justifyContent: "center", borderRadius: radius.md },
  pressed: { backgroundColor: colors.surfaceSubtle },
  disabled: { opacity: 0.4 },
  path: { color: colors.inkMuted, fontSize: 12, paddingHorizontal: layout.gutter, paddingVertical: spacing.sm },
  navigation: { flexDirection: "row", justifyContent: "space-between", paddingHorizontal: layout.gutter, borderBottomWidth: StyleSheet.hairlineWidth, borderBottomColor: colors.line },
  navButton: { minHeight: layout.touchTarget, flexDirection: "row", alignItems: "center", justifyContent: "center", gap: spacing.xs, paddingHorizontal: spacing.sm, borderRadius: radius.md },
  navLabel: { color: colors.inkMuted, fontSize: 14 },
  list: { paddingHorizontal: layout.gutter, paddingBottom: spacing.lg },
  row: { minHeight: 60, flexDirection: "row", alignItems: "center", gap: spacing.md, paddingVertical: spacing.sm, borderBottomWidth: StyleSheet.hairlineWidth, borderBottomColor: colors.lineSoft },
  name: { flex: 1, color: colors.ink, fontSize: 15 },
  size: { color: colors.inkMuted, fontSize: 12 },
  empty: { flexGrow: 1, alignItems: "center", justifyContent: "center", gap: spacing.md, padding: spacing.lg },
  emptyContent: { alignItems: "center", gap: spacing.md },
  status: { color: colors.inkMuted, textAlign: "center", fontSize: 14 },
});
