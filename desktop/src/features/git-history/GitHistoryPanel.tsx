import type { GraphGeometry } from "@future-os/git-graph";
import type { GitHistorySource } from "./gitHistorySource";
import type { GitCommitDetail, GitGraphListing, GitGraphRow } from "./types";
import {
  DARK_GRAPH_COLORS,
  graphGeometry,
  laneCount,
  LIGHT_GRAPH_COLORS,
  patchLines,
} from "@future-os/git-graph";
import { ChevronLeft, GitBranch, Loader2, RefreshCw } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
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

  const commits = useMemo(() => listing?.commits ?? [], [listing]);
  const lanes = useMemo(() => Math.max(1, laneCount(commits)), [commits]);
  // Decoded once for the page, not once per row: a row cannot be decoded alone
  // anyway — whether a thread reaches above a dot depends on the row before it.
  const geometry = useMemo(() => graphGeometry(commits, palette), [commits, palette]);
  const pictureWidth = lanes * LANE_PITCH + GUTTER_PAD;

  if (selected) {
    return <CommitPatchPane commit={selected} source={source} onBack={() => setSelected(null)} />;
  }

  return (
    <div className="flex h-full min-h-0 flex-col">
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
                        palette={palette}
                        width={pictureWidth}
                        onOpen={() => setSelected(commit)}
                      />
                    ))}
                    {listing?.truncated
                      ? (
                          <div className="px-4 py-3 text-center text-[11px] text-ink-muted">
                            {t("truncated")}
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
  palette,
  width,
  onOpen,
}: {
  commit: GitGraphRow;
  geometry: GraphGeometry | null;
  palette: readonly string[];
  width: number;
  onOpen: () => void;
}) {
  const { t } = useTranslation("gitHistory");
  const lane = geometry?.marker?.lane ?? 0;
  const stripe = palette[lane % palette.length] ?? palette[0]!;
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
        A hairline of the lane's colour at the row's left edge: what makes a page
        of commits scannable without reading the picture.
      */}
      <span
        aria-hidden="true"
        className="absolute inset-y-0 left-0 w-[2px] opacity-90"
        data-lane-stripe=""
        style={{ backgroundColor: stripe }}
      />
      <LaneGraph geometry={geometry} height={ROW_HEIGHT} width={width} center={DOT_CENTER} />
      <span className="min-w-0 flex-1">
        <span className="block truncate text-[13px] leading-[17px] text-ink">
          {commit.decorations
            ? <span className="font-semibold text-accent">{`${commit.decorations} `}</span>
            : null}
          {commit.subject}
        </span>
        <span className="block truncate text-[11px] leading-[15px] text-ink-muted">
          {commit.who}
          {` · ${commit.short} · ${commit.when}`}
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
            {`${commit.who} · `}
            <span className="font-mono">{commit.short}</span>
            {` · ${commit.when}`}
          </div>
        </div>
      </div>
      <div className="min-h-0 flex-1 overflow-auto border-t border-line-soft/70 font-mono text-[11px] leading-4">
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

function roleClass(role: "added" | "removed" | "hunk" | "header" | "meta" | "plain"): string {
  switch (role) {
    case "added": {
      return "text-success";
    }
    case "removed": {
      return "text-danger";
    }
    case "hunk": {
      return "text-info";
    }
    case "header": {
      return "font-semibold text-ink-soft";
    }
    case "meta": {
      return "text-ink-muted";
    }
    default: {
      return "text-ink-soft";
    }
  }
}
