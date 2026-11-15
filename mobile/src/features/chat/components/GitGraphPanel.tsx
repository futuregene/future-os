import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  ActivityIndicator,
  BackHandler,
  FlatList,
  Platform,
  Pressable,
  RefreshControl,
  ScrollView,
  StyleSheet,
  Text,
  View,
} from "react-native";
import Svg, { Circle, Path, Line as SvgLine } from "react-native-svg";
import type { GitCommitDetail, GitCommitRefs, GitGraphListing, GitGraphRow } from "../../../remote/types";
import { layout, radius, spacing } from "../../../theme/tokens";
import { createThemedStyles, useTheme } from "../../../theme/ThemeContext";
import {
  graphGeometry,
  laneCount,
  laneDrawing,
  patchLines,
  type GraphGeometry,
  type LaneMetrics,
  type PatchLine,
} from "../gitGraph";

/** A lane's width. Wide enough for a merge to read as a turn rather than a
 * kink, narrow enough that a six-lane history still fits a phone. */
const LANE_PITCH = 14;
const TURN_RADIUS = 6;
/** How much room a crossing opens around the lane it passes. */
const CROSSING_GAP = 3;
const THREAD_WIDTH = 2;
/** The commit's dot. A merge's is a ring, so `o` and `M` stay telling apart. */
const DOT_RADIUS = 3.75;
const DOT_STROKE = 1.75;
/** Room left and right of the picture, so a dot on lane 0 is not clipped. */
const GUTTER_PAD = LANE_PITCH / 2;
/** The row's floor height. It grows with the reader's text size — but it is one
 * number for the whole page, because the pictures only join up if every row is
 * drawn at the same height. */
const ROW_HEIGHT = 34;

const monospace = Platform.select({ ios: "Menlo", default: "monospace" });

/** The subject's line height, at the reader's text size. The row's picture puts
 * the commit's dot on this line, so the two are read together. */
const SUBJECT_LINE = 19;
/** The metadata line's height. It and the subject's are what the picture's axis
 * is computed from — see `CommitRow`. */
const META_LINE = 14;

/**
 * The conversation workspace's Git history: one row per commit, a lane per
 * column of work, and a tap opening that commit's patch.
 *
 * The lanes are drawn, not written. The host sends the picture as a grid of
 * box-drawing glyphs — the same string the TUI paints — and that grid is decoded
 * into strokes here: verticals that run the row's full height so they meet the
 * rows above and below, turns drawn as one curve, and crossings opened up so a
 * passing line does not appear to touch one that continues. The glyphs are never
 * rendered themselves: at phone size a monospace gutter reads as a column of
 * broken marks rather than as lines.
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
  registerBack,
}: {
  online: boolean;
  supported: boolean;
  listCommits: () => Promise<GitGraphListing>;
  getCommit: (gitRef: string) => Promise<GitCommitDetail>;
  onClose: () => void;
  /**
   * Hands the conversation's own back control a way to step back one level
   * inside this surface. The bar above already carries the only back arrow on
   * screen, so a second one here would be the same gesture twice in the same
   * corner: `true` means the press was taken (a patch was popped back to the
   * list), `false` means there is nothing left to pop and the caller should
   * leave the history.
   */
  registerBack?: (handler: () => boolean) => void;
}) {
  const styles = useStyles();
  const { colors, graphColors, font, leading } = useTheme();
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
    // The bar's back button steps back one level here before it leaves, so the
    // list is reached the same way a system back reaches it.
    registerBack?.(() => {
      if (!selected) return false;
      setSelected(null);
      return true;
    });
  }, [registerBack, selected]);

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

  const commits = useMemo(() => listing?.commits ?? [], [listing]);
  const lanes = useMemo(() => Math.max(1, laneCount(commits)), [commits]);
  // Decoded once per page rather than once per row: it is one linear pass, and
  // a row cannot be decoded alone anyway — whether a thread exists above a dot
  // depends on the row before it.
  const geometry = useMemo(() => graphGeometry(commits, graphColors), [commits, graphColors]);
  const rowHeight = Math.max(ROW_HEIGHT, leading(SUBJECT_LINE) + font(11) + 8);
  // Where the commit dot goes: the centre of the row's *first* line, not the
  // row's middle. The row stacks a subject and a metadata line, so its middle is
  // the gap between them — a dot there sits beside neither. Derived from the two
  // line heights rather than written as a number, because both scale with the
  // reader's text size and the row's own padding is what centres the pair.
  const dotCenter = (rowHeight - (leading(SUBJECT_LINE) + leading(META_LINE))) / 2
    + leading(SUBJECT_LINE) / 2;
  const metrics = useMemo<LaneMetrics>(
    () => ({ pitch: LANE_PITCH, radius: TURN_RADIUS, gap: CROSSING_GAP }),
    [],
  );
  const pictureWidth = lanes * LANE_PITCH + GUTTER_PAD;

  return (
    <View style={styles.panel}>
      {/* No header and no back arrow on either page. The conversation's own bar
          is still on screen above this surface, and its back button already
          steps back through it: from a patch to the list, from the list out of
          the history (`registerBack` is how it knows which). A second arrow
          here would be the same control twice, one under the other, in the same
          corner.
          That is also why neither page draws a title bar: the commits, then the
          patch, are one continuous surface, and a bar would spend a row of a
          phone screen repeating what the page already is. */}

      {selected ? (
        <CommitDetailPane commit={selected} getCommit={getCommit} />
      ) : loading ? (
        <View style={styles.empty}>
          <ActivityIndicator color={colors.accent} />
          <Text style={styles.status}>{t("git.loading")}</Text>
        </View>
      ) : (
        <FlatList
          data={commits}
          keyExtractor={commit => commit.short}
          contentContainerStyle={commits.length === 0 ? styles.empty : styles.list}
          // The same gesture as the transcript's: the history is what the desktop
          // answers with now, and re-reading it is not a rare operation — which is
          // why the refresh button the header used to carry is not missed.
          refreshControl={
            <RefreshControl refreshing={loading} onRefresh={refresh} tintColor={colors.inkMuted} />
          }
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
          renderItem={({ item, index }) => (
            <CommitRow
              commit={item}
              lane={geometry[index]?.marker?.lane ?? 0}
              row={geometry[index] ?? null}
              height={rowHeight}
              center={dotCenter}
              width={pictureWidth}
              metrics={metrics}
              onPress={() => setSelected(item)}
            />
          )}
        />
      )}
    </View>
  );
}

/**
 * One commit: its lane picture, the refs it carries and its subject.
 *
 * The picture is a handful of SVG strokes rather than a run of glyphs — a
 * twelve-lane history drawn per cell would be thousands of native views, and
 * strokes are what let a vertical continue across rows instead of appearing as a
 * column of separate marks.
 */
function CommitRow({
  commit,
  row,
  lane,
  height,
  center,
  width,
  metrics,
  onPress,
}: {
  commit: GitGraphRow;
  row: GraphGeometry | null;
  /** The lane the commit sits on, for the row's spoken hint. The picture draws
   * it; nothing on the row is tinted with it. */
  lane: number;
  height: number;
  /** The y the commit's dot is drawn at: the centre of the first text line. */
  center: number;
  width: number;
  metrics: LaneMetrics;
  onPress: () => void;
}) {
  const styles = useStyles();
  const { t } = useTranslation();
  const drawing = useMemo(
    () => (row ? laneDrawing(row, metrics, height, center) : null),
    [row, metrics, height, center],
  );

  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={t("git.openCommit", { sha: commit.short, subject: commit.subject })}
      accessibilityHint={t("git.graphLabel", { sha: commit.short, lane: lane + 1 })}
      onPress={onPress}
      style={({ pressed }) => [styles.row, { height }, pressed && styles.pressed]}
    >
      {/* Hidden from assistive tech: the row's own label already names the
          commit, and a picture of lanes has no useful reading order. The lane
          number is in the row's hint instead. */}
      <View pointerEvents="none" style={{ width, height }}>
        <Svg width={width} height={height} importantForAccessibility="no">
          {drawing?.threads.map((thread, index) => (
            <SvgLine
              key={`t${index}`}
              x1={thread.x}
              y1={thread.y1}
              x2={thread.x}
              y2={thread.y2}
              stroke={thread.color}
              strokeWidth={THREAD_WIDTH}
              strokeLinecap="round"
            />
          ))}
          {drawing?.runs.map((run, index) => (
            <Path
              key={`r${index}`}
              d={run.path}
              stroke={run.color}
              strokeWidth={THREAD_WIDTH}
              strokeLinecap="round"
              fill="none"
            />
          ))}
          {drawing?.dots.map((dot, index) => (
            // A merge is a ring, a plain commit a disc — the same distinction the
            // TUI's `M` and `o` make, drawn the same way so a reader who knows
            // one reads the other.
            <Circle
              key={`d${index}`}
              cx={dot.x}
              cy={dot.y}
              r={dot.merged ? DOT_RADIUS + DOT_STROKE / 2 : DOT_RADIUS}
              fill={dot.merged ? "none" : dot.color}
              stroke={dot.color}
              strokeWidth={dot.merged ? DOT_STROKE : 0}
            />
          ))}
        </Svg>
      </View>
      <View style={styles.body}>
        {/* One line: a commit list is for finding a commit, and a second line
            halves how many fit on a screen. The detail page has the full
            subject when it matters. */}
        <Text numberOfLines={1} style={styles.subject}>
          {commit.decorations ? (
            <Text style={styles.decorations}>{commit.decorations} </Text>
          ) : null}
          {commit.subject}
        </Text>
        <Text numberOfLines={1} style={styles.meta}>
          {commit.when} · {commit.short} · {commit.who}
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
      {/* The patch page carries no header, so what the header used to say lives
          here: the commit and its line, then who wrote it and when, in the same
          order the list row uses. The short sha is selectable because it is the
          thing a reader copies out. */}
      <Text selectable style={styles.detailSubject}>{commit.subject}</Text>
      <Text style={styles.detailMeta}>
        {`${commit.when} · `}
        <Text selectable style={styles.detailSha}>{commit.short}</Text>
        {` · ${commit.who}`}
      </Text>
      <CommitRefs refs={state.detail.refs} />
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

/**
 * Every ref that points at this commit.
 *
 * The list row has one line and shows git's short decoration, which drops local
 * branches once a commit carries several. Here there is room: the branches it is
 * on, the remote-tracking branches that carry it, and the tags cut on it — each
 * on its own row, because "is this released?" and "is this pushed?" are
 * different questions and a single run-together line answers neither well.
 */
function CommitRefs({ refs }: { refs: GitCommitRefs }) {
  const styles = useStyles();
  const { t } = useTranslation();
  // `HEAD` is listed first when the checkout is this commit: it is the answer to
  // "what am I looking at", and it can be true with no branch at all.
  const rows: { key: string; label: string; names: string[] }[] = [];
  if (refs.head) {
    rows.push({ key: "head", label: t("git.ref.checkedOut"), names: [] });
  }
  if (refs.branches.length) {
    rows.push({ key: "branches", label: t("git.ref.branches"), names: refs.branches });
  }
  if (refs.remotes.length) {
    rows.push({ key: "remotes", label: t("git.ref.remotes"), names: refs.remotes });
  }
  if (refs.tags.length) {
    rows.push({ key: "tags", label: t("git.ref.tags"), names: refs.tags });
  }
  if (rows.length === 0) {
    return null;
  }
  return (
    <View style={styles.refs}>
      {rows.map(row => (
        <View key={row.key} style={styles.refRow}>
          <Text style={styles.refLabel}>{row.label}</Text>
          <View style={styles.refChips}>
            {row.names.length === 0
              ? <Text style={styles.refChip}>{t("git.ref.yes")}</Text>
              : row.names.map(name => (
                  <Text key={name} selectable style={styles.refChip}>{name}</Text>
                ))}
          </View>
        </View>
      ))}
    </View>
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
  pressed: { backgroundColor: theme.colors.surfaceSubtle },
  list: { paddingTop: spacing.sm, paddingBottom: spacing.lg },
  row: {
    flexDirection: "row",
    alignItems: "center",
    gap: spacing.sm,
    paddingLeft: spacing.sm,
    paddingRight: layout.gutter,
  },
  body: { flex: 1, minWidth: 0 },
  subject: { color: theme.colors.ink, fontSize: theme.font(14), lineHeight: theme.leading(19) },
  decorations: { color: theme.colors.accent, fontWeight: "600" },
  meta: { color: theme.colors.inkMuted, fontSize: theme.font(11), lineHeight: theme.leading(META_LINE) },
  detailSubject: { color: theme.colors.inkStrong, fontSize: theme.font(15), fontWeight: "700", lineHeight: theme.leading(22) },
  detailMeta: { color: theme.colors.inkMuted, fontSize: theme.font(11), marginTop: spacing.xs },
  detailSha: { color: theme.colors.inkSoft, fontFamily: monospace },
  // The refs block: a label per kind and its names as chips, so "released" and
  // "pushed" stay separate answers instead of one run-together string.
  refs: { marginTop: spacing.sm, gap: spacing.xs },
  refRow: { flexDirection: "row", alignItems: "flex-start", gap: spacing.sm },
  refLabel: { color: theme.colors.inkMuted, fontSize: theme.font(11), lineHeight: theme.leading(18), minWidth: 48 },
  refChips: { flex: 1, flexDirection: "row", flexWrap: "wrap", gap: spacing.xs },
  refChip: {
    color: theme.colors.accent,
    fontSize: theme.font(11),
    lineHeight: theme.leading(18),
    backgroundColor: theme.colors.accentSoft,
    borderRadius: radius.sm,
    paddingHorizontal: spacing.xs,
    overflow: "hidden",
  },
  empty: { flexGrow: 1, alignItems: "center", justifyContent: "center", gap: spacing.md, padding: spacing.lg },
  emptyContent: { alignItems: "center", gap: spacing.md },
  status: { color: theme.colors.inkMuted, textAlign: "center", fontSize: theme.font(14) },
  footer: { color: theme.colors.inkMuted, fontSize: theme.font(12), paddingVertical: spacing.md, textAlign: "center" },
  retry: { minHeight: layout.touchTarget, justifyContent: "center", paddingHorizontal: spacing.md, borderRadius: radius.md },
  retryLabel: { color: theme.colors.accent, fontSize: theme.font(14) },
  detail: { padding: layout.gutter, paddingBottom: spacing.lg },
  // The card is the frame; the *lines* carry the tint, so the colour runs the
  // full width of a row rather than stopping where the text does. The container
  // therefore has no padding of its own — each line pads itself.
  patch: {
    backgroundColor: theme.colors.surfaceSubtle,
    borderRadius: radius.md,
    marginTop: spacing.md,
    overflow: "hidden",
  },
  patchLine: {
    fontFamily: monospace,
    fontSize: theme.font(11),
    lineHeight: theme.leading(16),
    color: theme.colors.ink,
    paddingHorizontal: spacing.sm,
    // A blank patch line is a real line (context); it must keep its height.
    minHeight: theme.leading(16),
  },
  // A line's role is carried by a tinted band plus a solid bar at the left, not
  // by its text colour alone: on a phone the monospaced 11px text is small, and
  // a hue difference alone is hard to scan down a page of it. The bar is what a
  // reader follows with their thumb; the tint says which side of the diff they
  // are on without reading a single character.
  patchAdded: {
    color: theme.colors.success,
    backgroundColor: theme.colors.successSoft,
    borderLeftWidth: 2,
    borderLeftColor: theme.colors.success,
  },
  patchRemoved: {
    color: theme.colors.danger,
    backgroundColor: theme.colors.dangerSoft,
    borderLeftWidth: 2,
    borderLeftColor: theme.colors.danger,
  },
  patchHunk: {
    color: theme.colors.info,
    backgroundColor: theme.colors.infoSoft,
    borderLeftWidth: 2,
    borderLeftColor: theme.colors.info,
    marginTop: spacing.xs,
  },
  patchHeader: {
    color: theme.colors.inkSoft,
    fontWeight: "600",
    borderLeftWidth: 2,
    borderLeftColor: theme.colors.line,
  },
  patchMeta: { color: theme.colors.inkMuted, borderLeftWidth: 2, borderLeftColor: "transparent" },
  patchPlain: { color: theme.colors.inkSoft, borderLeftWidth: 2, borderLeftColor: "transparent" },
}));

type PanelStyles = ReturnType<typeof useStyles>;
