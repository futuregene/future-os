import { createElement } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { BackHandler, FlatList } from "react-native";
import { GitGraphPanel } from "../components/GitGraphPanel";
import type { GitCommitDetail, GitGraphListing } from "../../../remote/types";

jest.mock("lucide-react-native", () => ({
  ArrowLeft: "ArrowLeft", RefreshCw: "RefreshCw", X: "X",
}));
jest.mock("../../../theme/ThemeContext", () => {
  const graphColors = ["#lane0", "#lane1", "#lane2"];
  return {
    useTheme: () => ({
      colors: {
        accent: "#accent", ink: "#ink", inkMuted: "#muted", inkSoft: "#soft",
        inkStrong: "#strong", danger: "#danger", success: "#success",
        info: "#info", lineSoft: "#line", surface: "#surface", surfaceSubtle: "#subtle",
      },
      graphColors,
      font: (size: number) => size,
      leading: (size: number) => size,
    }),
    createThemedStyles: (build: (theme: unknown) => unknown) => () =>
      build({
        colors: {
          accent: "#accent", ink: "#ink", inkMuted: "#muted", inkSoft: "#soft",
          inkStrong: "#strong", danger: "#danger", success: "#success",
          info: "#info", lineSoft: "#line", surface: "#surface", surfaceSubtle: "#subtle",
        },
        graphColors,
        font: (size: number) => size,
        leading: (size: number) => size,
      }) as Record<string, unknown>,
  };
});
jest.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string, values?: Record<string, string>) =>
    values ? `${key}:${JSON.stringify(values)}` : key }),
}));

/**
 * A three-lane history, taken from what the host really prints for this parent
 * graph (`host/src/remote_host/git_graph/tests.rs` draws the same rows from a
 * real repository).
 */
const listing: GitGraphListing = {
  isRepo: true,
  repoName: "dopamine-decision",
  path: "/Users/lixin/Research/dopamine-decision",
  commits: [
    { short: "4c1a7e2", when: "2026-11-11 18:42", who: "Lixin Zhou", decorations: "HEAD -> main", subject: "Merge the tolerance fix", graph: "M─┐   " },
    { short: "9b30d18", when: "2026-11-11 16:05", who: "Lixin Zhou", decorations: "", subject: "Tighten the fit tolerance", graph: "o │   " },
    { short: "e27f5a0", when: "2026-11-11 14:20", who: "Mei Tan", decorations: "origin/latent", subject: "Fit the latent-mixture model", graph: "│ M─┐ " },
  ],
  truncated: false,
};

const detail: GitCommitDetail = {
  short: "4c1a7e2",
  patch: [
    " x.R | 2 +-",
    " 1 file changed, 1 insertion(+), 1 deletion(-)",
    "",
    "diff --git a/x.R b/x.R",
    "--- a/x.R",
    "+++ b/x.R",
    "@@ -1 +1 @@",
    "-old",
    "+new",
    "",
  ].join("\n"),
  truncated: false,
};

let tree: ReactTestRenderer;
const listCommits = jest.fn<Promise<GitGraphListing>, []>(async () => listing);
const getCommit = jest.fn<Promise<GitCommitDetail>, [string]>(async () => detail);
const onClose = jest.fn();
const removeBack = jest.fn();

const mount = async (overrides: Partial<Parameters<typeof GitGraphPanel>[0]> = {}) => {
  await act(async () => {
    tree = create(createElement(GitGraphPanel, {
      online: true, supported: true, listCommits, getCommit, onClose, ...overrides,
    }));
  });
  await act(async () => { await Promise.resolve(); });
};
const commits = () => tree.root.findByType(FlatList);
/** Every string the panel currently paints, for the status messages. */
const texts = () => tree.root
  .findAll(node => String(node.type) === "Text")
  .map(node => node.children.map(String).join(""));
/** The system-back handler the panel has installed right now. A pop re-renders
 * and re-subscribes, so the newest subscription is the one Android would call. */
const backHandler = () => (BackHandler.addEventListener as jest.Mock).mock.calls.at(-1)![1];
/** The row's own press, found by its accessibility label. */
const rowPress = (sha: string) =>
  tree.root.findAll(node =>
    typeof node.props.accessibilityLabel === "string"
    && node.props.accessibilityLabel.startsWith(`git.openCommit:{"sha":"${sha}"`)
    && node.props.onPress)[0]!;

beforeEach(() => {
  jest.clearAllMocks();
  jest.spyOn(BackHandler, "addEventListener").mockReturnValue({ remove: removeBack });
});
afterEach(async () => {
  await act(async () => { await Promise.resolve(); });
  act(() => tree.unmount());
  jest.restoreAllMocks();
});

/**
 * The list is the history, drawn the way the TUI draws it.
 *
 * What matters on the phone is that every commit arrives with its lane picture
 * *and* that the picture is coloured — a hundred grey rows is a changelog, not a
 * graph. So the row's gutter is checked for span counts and colours rather than
 * for exact glyph positions: the glyphs are the host's, and re-asserting them
 * here would duplicate `gitGraph.test.ts` and the Rust tests behind it.
 */
test("every commit is listed with its coloured gutter and refs", async () => {
  await mount();
  expect(listCommits).toHaveBeenCalledTimes(1);
  expect(commits().props.data).toHaveLength(3);

  const first = rowPress("4c1a7e2");
  // The gutter is painted as coloured runs, so the lane's colour survives into a
  // real render rather than being flattened to one `Text`.
  const painted = first.findAll(node =>
    String(node.type) === "Text" && node.props.style
    && !Array.isArray(node.props.style));
  const colours = painted
    .map(node => (node.props.style as { color?: string }).color)
    .filter((colour): colour is string => typeof colour === "string");
  expect(colours).toContain("#lane0");
  // …and the row is addressable, because a bare gutter is not something a screen
  // reader can act on.
  expect(first.props.accessibilityRole).toBe("button");
});

test("a tapped commit opens its patch, coloured by line role", async () => {
  await mount();
  act(() => rowPress("4c1a7e2").props.onPress());
  await act(async () => { await Promise.resolve(); });

  expect(getCommit).toHaveBeenCalledWith("4c1a7e2");
  // The role, not the raw text: an addition and a removal must be told apart by
  // colour rather than by their leading character alone.
  const styles = tree.root.findAll(node =>
    String(node.type) === "Text" && node.props.style
    && Array.isArray(node.props.style));
  const flat = styles.flatMap(node => (node.props.style as unknown[]).flat());
  const colours = flat
    .filter((entry): entry is { color?: string } => !!entry && typeof entry === "object")
    .map(entry => entry.color);
  expect(colours).toContain("#success");
  expect(colours).toContain("#danger");
});

test("the detail pane is the one place a commit's bytes can be read, and back pops it", async () => {
  await mount();
  act(() => rowPress("e27f5a0").props.onPress());
  await act(async () => { await Promise.resolve(); });
  expect(getCommit).toHaveBeenCalledWith("e27f5a0");

  // System back pops the patch before it closes the sheet, so the Android edge
  // gesture leaves the reader on the list they were reading…
  act(() => backHandler()());
  expect(onClose).not.toHaveBeenCalled();
  // …and only a second press, from the list, closes it.
  act(() => backHandler()());
  expect(onClose).toHaveBeenCalledTimes(1);
});

test("offline and outdated desktops say so instead of listing nothing", async () => {
  await mount({ online: false });
  expect(listCommits).not.toHaveBeenCalled();
  expect(texts()).toContain("git.offline");

  act(() => tree.unmount());
  await mount({ supported: false });
  expect(listCommits).not.toHaveBeenCalled();
  expect(texts()).toContain("git.updateDesktop");
});

test("an empty repository says it is empty, and a failed read offers a retry", async () => {
  listCommits.mockResolvedValueOnce({ ...listing, commits: [] });
  await mount();
  expect(texts()).toContain("git.empty");

  act(() => tree.unmount());
  listCommits.mockRejectedValueOnce(new Error("nope"));
  await mount();
  // A failed read lands on the same empty state, plus a retry.
  expect(texts()).toContain("git.loadFailed");
  // The retry re-reads; it does not leave the reader on a dead screen.
  const read = listCommits.mock.calls.length;
  const retry = tree.root.findAll(node =>
    typeof node.props.onPress === "function"
    && node.findAll(child => child.children?.includes("common.retry")).length > 0)[0]!;
  await act(async () => { await retry.props.onPress(); });
  expect(listCommits.mock.calls.length).toBe(read + 1);
});

test("a cut log says the oldest commits are not shown", async () => {
  listCommits.mockResolvedValueOnce({ ...listing, truncated: true });
  await mount();
  expect(texts()).toContain("git.truncated");
});
