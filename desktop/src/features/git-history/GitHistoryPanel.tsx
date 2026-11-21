import type { GraphGeometry } from "@future-os/git-graph";
import type { GitHistorySource } from "./gitHistorySource";
import type { GitCommitDetail, GitCommitRefs, GitGraphListing, GitGraphRow } from "./types";
import {
  DARK_GRAPH_COLORS,
  graphGeometry,
  laneCount,
  LIGHT_GRAPH_COLORS,
  patchLines,
} from "@future-os/git-graph";
import { ChevronLeft, GitBranch, Loader2, RefreshCw } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { IconButton } from "../../components/ui/IconButton";
import { errorMessage } from "../../lib/errors";
import { useResolvedTheme } from "../../lib/theme";
import { getGitCommit, gitSourceKey, listGitHistory } from "./gitHistorySource";
import { LaneGraph } from "./LaneGraph";
import { GUTTER_PAD, LANE_PITCH } from "./laneMetrics";

/**
 * The row's height. Fixed, because the lane pictures only join up if every row
 * is drawn at the same height — and because a commit list is for finding a
 * commit, so one line of subject is the right amount.
 */
const ROW_HEIGHT = 34;
/**
 * The two text lines' heights, in pixels.
 *
 * Written out rather than left to the font's own leading because the picture's
 * axis is computed from them: the commit's dot belongs on the *first* line, and
 * where that line's centre falls depends on how the pair of lines sits inside
 * the row.
 */
const SUBJECT_LINE = 17;
const META_LINE = 15;
/**
 * The y the row's horizontals run on and its dot is drawn at: the centre of the
 * first line, with the two lines centred in the row.
 */
const DOT_CENTER = (ROW_HEIGHT - (SUBJECT_LINE + META_LINE)) / 2 + SUBJECT_LINE / 2;

/**
 * The conversation's Git history, as lanes.
 *
 * One commit per row, a lane per column of work, and a click opening that
 * commit's patch. The picture is the same one the phone's sheet draws: both
 * decode the host's glyph grid through `@future-os/git-graph`, so a reader who
 * has learned to read one has learned to read the other.
 *
 * Which machine is being read is not this component's business — it asks a
 * `GitHistorySource`, and a local workspace and a paired desktop answer
 * identically. `isRepo` decides whether the entry exists at all, and it comes
 * from the same read, so a plain directory costs no extra probe.
 */
export function GitHistoryPanel({
  source,
  onClose,
}: {
  source: GitHistorySource;
  onClose?: () => void;
}) {
  const { t } = useTranslation("gitHistory");
  const theme = useResolvedTheme();
  const palette = theme === "dark" ? DARK_GRAPH_COLORS : LIGHT_GRAPH_COLORS;
  const [revision, setRevision] = useState(0);
  const [state, setState] = useState<{
    key: string;
    listing: GitGraphListing | null;
    failed: string | null;
  } | null>(null);
  const [selected, setSelected] = useState<GitGraphRow | null>(null);
  // The pages past the first. Keyed by the same read as the first page, so a
  // refresh (or another source) drops them rather than stitching two different
  // reads into one list.
  const [pages, setPages] = useState<{
    key: string;
    rows: GitGraphRow[];
    /** Whether the page read last said another one follows. */
    truncated: boolean;
    /** Where that page stopped; "" once the last one answered. */
    cursor: string;
    /**
     * A peer that predates paging answered with the first page again: nothing new
     * arrived but it still claims more. The panel stops offering a button that
     * cannot advance, and says the rest is not shown.
     */
    stale: boolean;
    loading: boolean;
    failed: boolean;
  } | null>(null);
  // One page in flight at a time: the button is disabled while a read is open,
  // but a second click can land before the state it set has been painted.
  const askingRef = useRef(false);

  const key = useMemo(() => `${gitSourceKey(source)}#${revision}`, [source, revision]);
  const settled = state?.key === key;
  const listing = settled ? state.listing : null;
  const failed = settled ? state.failed : null;
  const loading = !settled;

  useEffect(() => {
    let current = true;
    void listGitHistory(source).then(
      (next) => {
        if (current) {
          setState({ key, listing: next, failed: null });
        }
      },
      (error: unknown) => {
        if (current) {
          setState({ key, listing: null, failed: errorMessage(error) });
        }
      },
    );
    return () => {
      current = false;
    };
  }, [source, key]);

  const commits = useMemo(() => {
    const first = listing?.commits ?? [];
    const appended = pages?.key === key ? pages : null;
    return appended && appended.rows.length > 0 ? [...first, ...appended.rows] : first;
  }, [listing, pages, key]);
  // The pages past the first, when they belong to the read on screen.
  const appended = pages?.key === key ? pages : null;
  // Whether the log goes on: the page read last is the authority once there is
  // one, and the first page's own flag says so before that.
  const more = appended ? appended.truncated : listing?.truncated === true;

  /**
   * Append the next page.
   *
   * The cursor is where the page already on screen stopped: the host reads the
   * next page *from* it, so the appended rows continue the lanes above them
   * rather than starting every line again at the left edge — and the read costs
   * one page however far the reader has scrolled.
   */
  const loadMore = async () => {
    if (!listing || askingRef.current || !more) {
      return;
    }
    askingRef.current = true;
    const asked = key;
    const cursor = appended?.cursor ?? listing.cursor ?? "";
    setPages(prev => ({
      key: asked,
      rows: prev?.key === asked ? prev.rows : [],
      truncated: prev?.key === asked ? prev.truncated : more,
      cursor: prev?.key === asked ? prev.cursor : cursor,
      stale: prev?.key === asked ? prev.stale : false,
      loading: true,
      failed: false,
    }));
    try {
      const next = await listGitHistory(source, cursor);
      setPages((prev) => {
        const rows = prev?.key === asked ? prev.rows : [];
        // A commit that landed between the two reads shifts the page's start and
        // its row arrives twice; the sha is what says it is the same commit.
        const seen = new Set([...(listing.commits), ...rows].map(row => row.short));
        const fresh = next.commits.filter(row => !seen.has(row.short));
        // A peer that cannot page answers every request with the first page:
        // nothing new arrives, yet it still reports more. Stopping is what keeps
        // the button from doing nothing forever. Read off the rows rather than
        // the cursor, so it holds for a peer that sends no cursor at all.
        const stale = fresh.length === 0 && next.truncated;
        return {
          key: asked,
          rows: [...rows, ...fresh],
          truncated: stale ? false : next.truncated,
          cursor: next.cursor ?? "",
          stale,
          loading: false,
          failed: false,
        };
      });
    }
    catch {
      setPages(prev => ({
        key: asked,
        rows: prev?.key === asked ? prev.rows : [],
        truncated: prev?.key === asked ? prev.truncated : more,
        cursor: prev?.key === asked ? prev.cursor : cursor,
        stale: prev?.key === asked ? prev.stale : false,
        loading: false,
        failed: true,
      }));
    }
    finally {
      askingRef.current = false;
    }
  };

  const lanes = useMemo(() => Math.max(1, laneCount(commits)), [commits]);
  // Decoded once for the page, not once per row: a row cannot be decoded alone
  // anyway — whether a thread reaches above a dot depends on the row before it.
  const geometry = useMemo(() => graphGeometry(commits, palette), [commits, palette]);
  const pictureWidth = lanes * LANE_PITCH + GUTTER_PAD;

  if (selected) {
    return <CommitPatchPane commit={selected} source={source} onBack={() => setSelected(null)} />;
  }

  return (
    <div data-git-history={source.kind} className="flex h-full min-h-0 flex-col">
      <div className="flex shrink-0 items-center gap-2 px-3 py-2">
        {onClose
          ? <IconButton label={t("close")} onClick={onClose} icon={<ChevronLeft size={16} />} />
          : null}
        <div className="min-w-0 flex-1">
          <div className="truncate text-xs font-semibold text-ink-strong">
            {listing?.repoName || t("title")}
          </div>
          {listing?.path
            ? <div className="truncate text-[11px] text-ink-muted">{listing.path}</div>
            : null}
        </div>
        <IconButton
          label={t("refresh")}
          disabled={loading}
          onClick={() => setRevision(value => value + 1)}
          icon={<RefreshCw size={15} className={loading ? "animate-spin" : undefined} />}
        />
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto border-t border-line-soft/70">
        {loading
          ? (
              <div className="flex items-center justify-center gap-2 px-4 py-8 text-xs text-ink-muted">
                <Loader2 size={14} className="animate-spin" />
                {t("loading")}
              </div>
            )
          : failed
            ? <div className="px-4 py-6 text-center text-xs text-ink-muted">{failed}</div>
            : commits.length === 0
              ? (
                  <div className="flex flex-col items-center gap-2 px-4 py-8 text-center">
                    <GitBranch size={18} className="text-ink-muted" />
                    <div className="text-xs text-ink-muted">
                      {listing?.isRepo ? t("empty") : t("notRepo")}
                    </div>
                  </div>
                )
              : (
                  <>
                    {commits.map((commit, index) => (
                      <CommitRow
                        key={commit.short}
                        commit={commit}
                        geometry={geometry[index] ?? null}
                        width={pictureWidth}
                        onOpen={() => setSelected(commit)}
                      />
                    ))}
                    {more
                      ? (
                          <div className="px-3 py-2">
                            <button
                              type="button"
                              data-git-load-more
                              onClick={() => { void loadMore(); }}
                              disabled={appended?.loading}
                              className="flex w-full items-center justify-center gap-1.5 rounded px-3 py-2 text-[11px] text-accent hover:bg-surface-subtle disabled:opacity-70"
                            >
                              {appended?.loading
                                ? <Loader2 size={12} className="animate-spin" />
                                : null}
                              {appended?.failed
                                ? t("loadMoreFailed")
                                : appended?.loading ? t("loadingMore") : t("loadMore")}
                            </button>
                          </div>
                        )
                      // Only once the reader has paged: a history that fit in one
                      // page has nothing to explain.
                      : appended?.stale
                        ? (
                            <div className="px-4 py-3 text-center text-[11px] text-ink-muted">
                              {t("truncated")}
                            </div>
                          )
                        : appended
                          ? (
                              <div className="px-4 py-3 text-center text-[11px] text-ink-muted">
                                {t("historyEnd")}
                              </div>
                            )
                          : null}
                  </>
                )}
      </div>
    </div>
  );
}

function CommitRow({
  commit,
  geometry,
  width,
  onOpen,
}: {
  commit: GitGraphRow;
  geometry: GraphGeometry | null;
  width: number;
  onOpen: () => void;
}) {
  const { t } = useTranslation("gitHistory");
  return (
    <button
      data-git-commit={commit.short}
      type="button"
      onClick={onOpen}
      aria-label={t("openCommit", { sha: commit.short, subject: commit.subject })}
      className="group relative flex w-full items-center gap-2 px-3 text-left hover:bg-surface-subtle"
      style={{ height: ROW_HEIGHT }}
    >
      {/*
        No lane hairline at the row's left edge. It was pinned to the row's edge
        rather than to the lane column, changed colour from row to row, and
        stopped short of the threads it echoed — so it read as a stray border
        beside the picture instead of as part of it. The lanes themselves say
        which column a commit is on.
      */}
      <LaneGraph geometry={geometry} height={ROW_HEIGHT} width={width} center={DOT_CENTER} />
      <span className="min-w-0 flex-1">
        <span className="block truncate text-[13px] leading-[17px] text-ink">
          {commit.decorations
            ? <span className="font-semibold text-accent">{`${commit.decorations} `}</span>
            : null}
          {commit.subject}
        </span>
        <span className="block truncate text-[11px] leading-[15px] text-ink-muted">
          {`${commit.when} · ${commit.short} · ${commit.who}`}
        </span>
      </span>
    </button>
  );
}

/**
 * One commit's stat and patch.
 *
 * Coloured by line *role* rather than by git's own ANSI: the escape codes would
 * ride along in the text the reader selects, and they say nothing about which
 * lines matter once the terminal codes are stripped.
 */
function CommitPatchPane({
  commit,
  source,
  onBack,
}: {
  commit: GitGraphRow;
  source: GitHistorySource;
  onBack: () => void;
}) {
  const { t } = useTranslation("gitHistory");
  const [state, setState] = useState<{
    key: string;
    detail: GitCommitDetail | null;
    failed: string | null;
  } | null>(null);
  const settled = state?.key === commit.short;

  useEffect(() => {
    let current = true;
    void getGitCommit(source, commit.short).then(
      (detail) => {
        if (current) {
          setState({ key: commit.short, detail, failed: null });
        }
      },
      (error: unknown) => {
        if (current) {
          setState({ key: commit.short, detail: null, failed: errorMessage(error) });
        }
      },
    );
    return () => {
      current = false;
    };
  }, [commit.short, source]);

  const lines = useMemo(
    () => (settled && state?.detail ? patchLines(state.detail.patch) : []),
    [settled, state],
  );

  return (
    <div className="flex h-full min-h-0 flex-col">
      <div className="flex shrink-0 items-center gap-2 px-3 py-2">
        <IconButton
          label={t("back")}
          onClick={onBack}
          icon={<ChevronLeft size={16} />}
          data-git-back=""
        />
        <div className="min-w-0 flex-1">
          <div className="truncate text-xs font-semibold text-ink-strong">{commit.subject}</div>
          <div className="truncate text-[11px] text-ink-muted">
            {`${commit.when} · `}
            <span className="font-mono">{commit.short}</span>
            {` · ${commit.who}`}
          </div>
        </div>
      </div>
      <div className="min-h-0 flex-1 overflow-auto border-t border-line-soft/70 font-mono text-[11px] leading-4">
        {settled && state?.detail
          ? <CommitRefs refs={state.detail.refs} />
          : null}
        {!settled
          ? (
              <div className="flex items-center justify-center gap-2 px-4 py-8 text-ink-muted">
                <Loader2 size={14} className="animate-spin" />
                {t("loading")}
              </div>
            )
          : state?.failed
            ? <div className="px-4 py-6 text-center text-ink-muted">{state.failed}</div>
            : (
                <div className="min-w-max py-1">
                  {lines.map((line, index) => (
                    <div
                      // A patch repeats lines constantly (blank context, `---`),
                      // so the position is what identifies one.
                      // eslint-disable-next-line react/no-array-index-key
                      key={`${index}:${line.text}`}
                      className={`whitespace-pre px-3 ${roleClass(line.role)}`}
                      data-patch-role={line.role}
                    >
                      {line.text === "" ? " " : line.text}
                    </div>
                  ))}
                </div>
              )}
      </div>
    </div>
  );
}

/**
 * Every ref pointing at the open commit.
 *
 * The row shows git's short decoration, which drops local branches once a commit
 * carries several. Here the whole set is listed, one kind per line: "is this
 * released?" (tags), "is it pushed?" (remotes) and "is it on a branch?" are
 * different questions, and a single run-together string answers none of them.
 */
function CommitRefs({ refs }: { refs: GitCommitRefs }) {
  const { t } = useTranslation("gitHistory");
  const rows: { key: string; label: string; names: string[] }[] = [];
  // `HEAD` first: it answers "what am I looking at", and it can be true with no
  // branch at all (a detached checkout of an old commit).
  if (refs.head)
    rows.push({ key: "head", label: t("ref.checkedOut"), names: [] });
  if (refs.branches.length)
    rows.push({ key: "branches", label: t("ref.branches"), names: refs.branches });
  if (refs.remotes.length)
    rows.push({ key: "remotes", label: t("ref.remotes"), names: refs.remotes });
  if (refs.tags.length)
    rows.push({ key: "tags", label: t("ref.tags"), names: refs.tags });
  if (rows.length === 0)
    return null;
  return (
    <div className="border-b border-line-soft/70 px-3 py-2 font-sans" data-git-refs="">
      {rows.map(row => (
        <div key={row.key} className="flex items-start gap-2 py-0.5 text-[11px]">
          <span className="shrink-0 text-ink-muted">{row.label}</span>
          <span className="flex min-w-0 flex-wrap gap-1">
            {row.names.length === 0
              ? <span className="rounded bg-accent-soft px-1 text-accent">{t("ref.yes")}</span>
              : row.names.map(name => (
                  <span key={name} className="select-text rounded bg-accent-soft px-1 text-accent">
                    {name}
                  </span>
                ))}
          </span>
        </div>
      ))}
    </div>
  );
}

function roleClass(role: "added" | "removed" | "hunk" | "header" | "meta" | "plain"): string {
  switch (role) {
    case "added": {
      return "border-l-2 border-success bg-success-soft text-success";
    }
    case "removed": {
      return "border-l-2 border-danger bg-danger-soft text-danger";
    }
    case "hunk": {
      return "border-l-2 border-info bg-info-soft text-info";
    }
    case "header": {
      return "border-l-2 border-line font-semibold text-ink-soft";
    }
    case "meta": {
      return "border-l-2 border-transparent text-ink-muted";
    }
    default: {
      return "border-l-2 border-transparent text-ink-soft";
    }
  }
}
