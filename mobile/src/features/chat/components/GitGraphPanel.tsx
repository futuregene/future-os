import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { ArrowLeft, RefreshCw, X } from "lucide-react-native";
import {
  ActivityIndicator,
  BackHandler,
  FlatList,
  Platform,
  Pressable,
  ScrollView,
  StyleSheet,
  Text,
  View,
} from "react-native";
import type { GitCommitDetail, GitGraphListing, GitGraphRow } from "../../../remote/types";
import { layout, radius, spacing } from "../../../theme/tokens";
import { createThemedStyles, useTheme } from "../../../theme/ThemeContext";
import { commitLane, graphSpans, patchLines, type PatchLine } from "../gitGraph";

/** The metrics the gutter is laid out with. A lane is two cells and a
 * connection is drawn *between* them, so the picture only lines up when every
 * cell is the same width — which is why this whole surface is monospace. */
const monospace = Platform.select({ ios: "Menlo", default: "monospace" });

/**
 * The conversation workspace's Git history, as the TUI's `/git` panel draws it:
 * one row per commit, a lane per column of work, and a tap opening that commit's
 * patch.
 *
 * An in-screen surface rather than a native Modal, for the same reason
 * `SessionFilesPanel` is: file previews and download dialogs can present above it
 * on iOS without competing UIKit modal presentations.
 */
export function GitGraphPanel({
  online,
  supported,
  listCommits,
  getCommit,
  onClose,
}: {
  online: boolean;
  supported: boolean;
  listCommits: () => Promise<GitGraphListing>;
  getCommit: (gitRef: string) => Promise<GitCommitDetail>;
  onClose: () => void;
}) {
  const styles = useStyles();
  const { colors } = useTheme();
  const { t } = useTranslation();
  const [revision, setRevision] = useState(0);
  const [result, setResult] = useState<{
    request: object;
    listing: GitGraphListing | null;
    failed: boolean;
  } | null>(null);
  const [selected, setSelected] = useState<GitGraphRow | null>(null);

  // One identity object per read, so an answer that arrives after the sheet was
  // refreshed (or after the reader opened another commit) is discarded rather
  // than painted over the newer state.
  const request = useMemo(
    () => ({ listCommits, revision, online, supported }),
    [listCommits, revision, online, supported],
  );
  const available = online && supported;
  const settled = result?.request === request;
  const listing = available && settled ? result.listing : null;
  const failed = available && settled && result.failed;
  const loading = available && !settled;

  useEffect(() => {
    let current = true;
    if (request.online && request.supported) {
      void request.listCommits().then(
        next => { if (current) setResult({ request, listing: next, failed: false }); },
        () => { if (current) setResult({ request, listing: null, failed: true }); },
      );
    }
    return () => { current = false; };
  }, [request]);

  useEffect(() => {
    // A commit's detail owns system back while it is open, so Android's edge
    // gesture leaves the patch rather than the whole sheet.
    const subscription = BackHandler.addEventListener("hardwareBackPress", () => {
      if (selected) setSelected(null);
      else onClose();
      return true;
    });
    return () => subscription.remove();
  }, [onClose, selected]);

  const refresh = () => {
    setSelected(null);
    setRevision(value => value + 1);
  };
  const status = !online ? t("git.offline")
    : !supported ? t("git.updateDesktop")
      : failed ? t("git.loadFailed")
        : t("git.empty");

  return (
    <View style={styles.panel}>
      <View style={styles.toolbar}>
        {selected ? (
          <Pressable
            accessibilityRole="button"
            accessibilityLabel={t("git.back")}
            onPress={() => setSelected(null)}
            style={({ pressed }) => [styles.iconButton, pressed && styles.pressed]}
          >
            <ArrowLeft color={colors.inkSoft} size={20} />
          </Pressable>
        ) : (
          <View style={styles.iconButton} />
        )}
        <View style={styles.headingWrap}>
          <Text numberOfLines={1} style={styles.heading}>
            {selected ? selected.short : t("git.title")}
          </Text>
          {!selected && listing?.isRepo ? (
            <Text numberOfLines={1} ellipsizeMode="middle" style={styles.subheading}>
              {t("git.header", { name: listing.repoName, path: listing.path })}
            </Text>
          ) : selected ? (
            <Text numberOfLines={1} style={styles.subheading}>
              {selected.who} · {selected.when}
            </Text>
          ) : null}
        </View>
        <Pressable
          accessibilityRole="button"
          accessibilityLabel={selected ? t("common.close") : t("git.refresh")}
          accessibilityState={{ disabled: !selected && (!available || loading) }}
          disabled={!selected && (!available || loading)}
          onPress={selected ? onClose : refresh}
          style={({ pressed }) => [
            styles.iconButton,
            pressed && styles.pressed,
            !selected && (!available || loading) && styles.disabled,
          ]}
        >
          {selected ? (
            <X color={colors.inkSoft} size={20} />
          ) : (
            <RefreshCw size={20} color={colors.inkMuted} />
          )}
        </Pressable>
      </View>

      {selected ? (
        <CommitDetailPane commit={selected} getCommit={getCommit} />
      ) : loading ? (
        <View style={styles.empty}>
          <ActivityIndicator color={colors.accent} />
          <Text style={styles.status}>{t("git.loading")}</Text>
        </View>
      ) : (
        <FlatList
          data={listing?.commits ?? []}
          keyExtractor={commit => commit.short}
          contentContainerStyle={(listing?.commits.length ?? 0) === 0 ? styles.empty : styles.list}
          ListEmptyComponent={
            <View style={styles.emptyContent}>
              <Text style={styles.status}>{status}</Text>
              {failed && available && (
                <Pressable accessibilityRole="button" onPress={refresh} style={styles.retry}>
                  <Text style={styles.retryLabel}>{t("common.retry")}</Text>
                </Pressable>
              )}
            </View>
          }
          ListFooterComponent={
            listing?.truncated ? <Text style={styles.footer}>{t("git.truncated")}</Text> : undefined
          }
          renderItem={({ item }) => <CommitRow commit={item} onPress={() => setSelected(item)} />}
        />
      )}
    </View>
  );
}

/** One commit: its lanes, then the refs it carries and its subject.
 *
 * The gutter is one `Text` per colour run, not one per cell: a page is up to 200
 * rows and a wide history is a dozen-lane gutter, so a per-cell `Text` would be
 * thousands of native views for one screen. Runs are merged in `graphSpans`,
 * which is also where the lane → colour rule lives. */
function CommitRow({ commit, onPress }: { commit: GitGraphRow; onPress: () => void }) {
  const styles = useStyles();
  const { colors, graphColors } = useTheme();
  const { t } = useTranslation();
  const spans = useMemo(() => graphSpans(commit.graph, graphColors), [commit.graph, graphColors]);
  const lane = commitLane(commit.graph);
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={t("git.openCommit", { sha: commit.short, subject: commit.subject })}
      onPress={onPress}
      style={({ pressed }) => [styles.row, pressed && styles.pressed]}
    >
      {/* The stripe is what makes a page of commits scannable: the eye follows
          one colour down the list without reading a glyph. It is decoration, so
          it carries no name of its own — the row's label says which commit and
          the gutter text underneath is read as part of the row. */}
      <View
        pointerEvents="none"
        style={[styles.laneStripe, { backgroundColor: graphColors[lane % graphColors.length] }]}
      />
      <Text style={[styles.graph, { color: colors.inkMuted }]} importantForAccessibility="no">
        {spans.map((span, index) => (
          <Text key={index} style={span.color ? { color: span.color } : undefined}>
            {span.text}
          </Text>
        ))}
      </Text>
      <View style={styles.body}>
        <Text numberOfLines={2} style={styles.subject}>
          {commit.decorations ? (
            <Text style={styles.decorations}>{commit.decorations} </Text>
          ) : null}
          {commit.subject}
        </Text>
        <Text numberOfLines={1} style={styles.meta}>
          {commit.short} · {commit.who} · {commit.when}
        </Text>
      </View>
    </Pressable>
  );
}

/** One commit's stat and patch, coloured by line role rather than by git's own
 * ANSI (which would ride along in the text the reader copies). */
function CommitDetailPane({
  commit,
  getCommit,
}: {
  commit: GitGraphRow;
  getCommit: (gitRef: string) => Promise<GitCommitDetail>;
}) {
  const styles = useStyles();
  const { colors } = useTheme();
  const { t } = useTranslation();
  const [state, setState] = useState<{
    key: string;
    detail: GitCommitDetail | null;
    failed: boolean;
  } | null>(null);
  const settled = state?.key === commit.short;

  useEffect(() => {
    let current = true;
    void getCommit(commit.short).then(
      detail => { if (current) setState({ key: commit.short, detail, failed: false }); },
      () => { if (current) setState({ key: commit.short, detail: null, failed: true }); },
    );
    return () => { current = false; };
  }, [commit.short, getCommit]);

  const lines = useMemo(
    () => (settled && state?.detail ? patchLines(state.detail.patch) : []),
    [settled, state],
  );

  if (!settled) {
    return (
      <View style={styles.empty}>
        <ActivityIndicator color={colors.accent} />
        <Text style={styles.status}>{t("git.loading")}</Text>
      </View>
    );
  }
  if (state?.failed || !state?.detail) {
    return (
      <View style={styles.empty}>
        <Text style={styles.status}>{t("git.commitFailed")}</Text>
      </View>
    );
  }
  return (
    <ScrollView contentContainerStyle={styles.detail}>
      <Text selectable style={styles.detailSubject}>{commit.subject}</Text>
      <View style={styles.patch}>
        {lines.map((line, index) => (
          <Text key={index} selectable style={[styles.patchLine, roleStyle(styles, line.role)]}>
            {line.text === "" ? " " : line.text}
          </Text>
        ))}
      </View>
    </ScrollView>
  );
}

function roleStyle(styles: PanelStyles, role: PatchLine["role"]) {
  switch (role) {
    case "added": return styles.patchAdded;
    case "removed": return styles.patchRemoved;
    case "hunk": return styles.patchHunk;
    case "header": return styles.patchHeader;
    case "meta": return styles.patchMeta;
    default: return styles.patchPlain;
  }
}

const useStyles = createThemedStyles(theme => StyleSheet.create({
  panel: { position: "absolute", top: 0, bottom: 0, left: 0, right: 0, zIndex: 4, backgroundColor: theme.colors.surface },
  toolbar: { flexDirection: "row", alignItems: "center", paddingHorizontal: layout.gutter, paddingTop: spacing.sm, gap: spacing.xs },
  headingWrap: { flex: 1, minWidth: 0 },
  heading: { color: theme.colors.inkStrong, fontSize: theme.font(18), fontWeight: "700" },
  subheading: { color: theme.colors.inkMuted, fontSize: theme.font(11), marginTop: 1 },
  iconButton: { width: layout.touchTarget, height: layout.touchTarget, alignItems: "center", justifyContent: "center", borderRadius: radius.md },
  pressed: { backgroundColor: theme.colors.surfaceSubtle },
  disabled: { opacity: 0.4 },
  list: { paddingBottom: spacing.lg },
  row: {
    flexDirection: "row",
    alignItems: "flex-start",
    gap: spacing.sm,
    paddingVertical: spacing.sm,
    paddingLeft: spacing.sm,
    paddingRight: layout.gutter,
    borderBottomWidth: StyleSheet.hairlineWidth,
    borderBottomColor: theme.colors.lineSoft,
  },
  // The lane's colour as a hairline at the row's left edge. Two points: enough to
  // follow a line of work while scrolling, small enough not to compete with the
  // gutter's own glyphs.
  laneStripe: { position: "absolute", left: 0, top: 0, bottom: 0, width: 2, opacity: 0.9 },
  graph: {
    fontFamily: monospace,
    fontSize: theme.font(12),
    lineHeight: theme.leading(18),
    // A fixed grid: the gutter never wraps and never shrinks, or the lanes stop
    // lining up with the row above.
    flexShrink: 0,
  },
  body: { flex: 1, minWidth: 0 },
  subject: { color: theme.colors.ink, fontSize: theme.font(14), lineHeight: theme.leading(20) },
  decorations: { color: theme.colors.accent, fontWeight: "600" },
  meta: { color: theme.colors.inkMuted, fontSize: theme.font(11), marginTop: 2 },
  detailSubject: { color: theme.colors.inkStrong, fontSize: theme.font(15), fontWeight: "700", lineHeight: theme.leading(22) },
  empty: { flexGrow: 1, alignItems: "center", justifyContent: "center", gap: spacing.md, padding: spacing.lg },
  emptyContent: { alignItems: "center", gap: spacing.md },
  status: { color: theme.colors.inkMuted, textAlign: "center", fontSize: theme.font(14) },
  footer: { color: theme.colors.inkMuted, fontSize: theme.font(12), paddingVertical: spacing.md, textAlign: "center" },
  retry: { minHeight: layout.touchTarget, justifyContent: "center", paddingHorizontal: spacing.md, borderRadius: radius.md },
  retryLabel: { color: theme.colors.accent, fontSize: theme.font(14) },
  detail: { padding: layout.gutter, paddingBottom: spacing.lg },
  patch: { backgroundColor: theme.colors.surfaceSubtle, borderRadius: radius.md, padding: spacing.sm, marginTop: spacing.md },
  patchLine: { fontFamily: monospace, fontSize: theme.font(11), lineHeight: theme.leading(16), color: theme.colors.ink },
  patchAdded: { color: theme.colors.success },
  patchRemoved: { color: theme.colors.danger },
  patchHunk: { color: theme.colors.info },
  patchHeader: { color: theme.colors.inkSoft, fontWeight: "600" },
  patchMeta: { color: theme.colors.inkMuted },
  patchPlain: { color: theme.colors.inkSoft },
}));

type PanelStyles = ReturnType<typeof useStyles>;
