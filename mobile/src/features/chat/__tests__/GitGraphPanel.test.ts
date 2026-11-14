import { createElement } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { BackHandler, FlatList } from "react-native";
import Svg, { Circle, Line as SvgLine } from "react-native-svg";
import { GitGraphPanel } from "../components/GitGraphPanel";
import type { GitCommitDetail, GitGraphListing } from "../../../remote/types";

jest.mock("lucide-react-native", () => ({ ArrowLeft: "ArrowLeft" }));
jest.mock("../../../theme/ThemeContext", () => {
  // Real hex, not placeholders: `react-native-svg` parses the colour and warns
  // on anything it cannot read, so a fake palette would exercise a path the app
  // never takes.
  const graphColors = ["#2563eb", "#0e7490", "#a16207"];
  const theme = {
    colors: {
      accent: "#accent", ink: "#ink", inkMuted: "#muted", inkSoft: "#soft",
      inkStrong: "#strong", danger: "#danger", success: "#success",
      info: "#info", lineSoft: "#line", surface: "#surface", surfaceSubtle: "#subtle",
    },
    graphColors,
    font: (size: number) => size,
    leading: (size: number) => size,
  };
  return {
    useTheme: () => theme,
    createThemedStyles: (build: (value: unknown) => unknown) => () => build(theme),
  };
});
jest.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string, values?: Record<string, string>) =>
    values ? `${key}:${JSON.stringify(values)}` : key }),
}));

/**
 * The gutters are what the host really prints for this parent graph, taken from
 * `remote_host::git_graph::list` against a repository built by the same fixture
 * the Rust tests use — two branches merged back into `main`. Nothing here is
 * invented, so a change in the decoder shows up against the desktop's real
 * output rather than against a picture typed out by the test.
 */
const listing: GitGraphListing = {
  isRepo: true,
  repoName: "dopamine-decision",
  path: "/Users/lixin/Research/dopamine-decision",
  commits: [
    { short: "4c1a7e2", when: "2026-11-11 18:42", who: "Lixin Zhou", decorations: "", subject: "diverge", graph: "o     " },
    { short: "9b30d18", when: "2026-11-11 16:05", who: "Lixin Zhou", decorations: "HEAD -> main", subject: "merge side", graph: "│ M─┐ " },
    { short: "e27f5a0", when: "2026-11-11 14:20", who: "Mei Tan", decorations: "origin/side", subject: "side work", graph: "│ │ o " },
    { short: "1a2b3c4", when: "2026-11-11 12:00", who: "Mei Tan", decorations: "", subject: "main work", graph: "│ o │ " },
    { short: "5d6e7f8", when: "2026-11-11 09:30", who: "Lixin Zhou", decorations: "", subject: "base", graph: "o─┴─┘ " },
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
/** One node's text with its nested `Text` children in place — the order a reader
 * sees, which a per-node join cannot show because the pieces are separate
 * nodes. */
const textOf = (node: { children: unknown[] }): string => node.children
  .map((child) => {
    if (typeof child === "string") return child;
    if (child && typeof child === "object" && "children" in child) {
      return textOf(child as { children: unknown[] });
    }
    return "";
  })
  .join("");
/** The system-back handler the panel has installed right now. A pop re-renders
 * and re-subscribes, so the newest subscription is the one Android would call. */
const backHandler = () => (BackHandler.addEventListener as jest.Mock).mock.calls.at(-1)![1];
/** A row's own press, found by its accessibility label. */
const rowPress = (sha: string) =>
  tree.root.findAll(node =>
    typeof node.props.accessibilityLabel === "string"
    && node.props.accessibilityLabel.startsWith(`git.openCommit:{"sha":"${sha}"`)
    && node.props.onPress)[0]!;
/** The way out of whichever page is showing — `Back to commits` on a patch,
 * `Close` on the list. Found by label so the test does not depend on the icon. */
const escapeButton = (label: string) =>
  tree.root.findAll(node =>
    node.props.accessibilityRole === "button"
    && node.props.accessibilityLabel === label
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
 * The list is the history, and the lanes are *drawn*.
 *
 * The desktop sends the picture as a grid of box-drawing glyphs; painting those
 * glyphs is what makes a phone-sized graph look like a column of broken marks.
 * So the row is checked for strokes — lines, paths and dots — and for the lane
 * colours on them. What is asserted is the shape of the drawing, not the exact
 * path data: `gitGraph.test.ts` pins the numbers, and re-asserting them per row
 * here would mean editing two files for every tweak to the corner radius.
 */
test("every commit is listed with its lane drawn in colour", async () => {
  await mount();
  expect(listCommits).toHaveBeenCalledTimes(1);
  expect(commits().props.data).toHaveLength(5);

  const first = rowPress("4c1a7e2");
  // The picture is a picture: no glyph from the host's gutter survives into the
  // rendered tree.
  const painted = first.findAll(node => String(node.type) === "Text")
    .flatMap(node => node.children.map(String))
    .join("");
  expect(painted).not.toMatch(/[│─┐┘┴┬┤]/);
  // …and it is made of strokes rather than of characters.
  expect(first.findAllByType(Svg)).toHaveLength(1);
  const strokes = first.findAllByType(SvgLine).map(node => node.props.stroke);
  expect(strokes.length).toBeGreaterThan(0);
  // Colour reaches the strokes: the thread is drawn in the lane's own colour
  // rather than in the theme's default ink.
  expect(strokes).toContain("#2563eb");
  // The row is addressable, because a drawing is not something a screen reader
  // can act on.
  expect(first.props.accessibilityRole).toBe("button");
  // The row is addressable, because a drawing is not something a screen reader
  // can act on.
  expect(first.props.accessibilityRole).toBe("button");
});

test("a merge is drawn as a ring and a plain commit as a disc", async () => {
  await mount();
  const discs = rowPress("4c1a7e2").findAllByType(Circle);
  expect(discs).toHaveLength(1);
  expect(discs[0]!.props.fill).not.toBe("none");

  const rings = rowPress("9b30d18").findAllByType(Circle);
  expect(rings).toHaveLength(1);
  expect(rings[0]!.props.fill).toBe("none");
});

/** Neither page has a header bar: one control, top-left, and the content. */
test("the list has no header: just the way out", async () => {
  await mount();
  // Nothing names the repository at the top of the list; the commits are the
  // page.
  expect(texts()).not.toContain("dopamine-decision");
  expect(texts()).not.toContain("/Users/lixin/Research/dopamine-decision");
  // The one control is the back button, labelled for leaving.
  expect(escapeButton("common.close")).toBeDefined();
  act(() => escapeButton("common.close").props.onPress());
  expect(onClose).toHaveBeenCalledTimes(1);
});

test("a tapped commit opens its patch, coloured by line role", async () => {
  await mount();
  act(() => rowPress("4c1a7e2").props.onPress());
  await act(async () => { await Promise.resolve(); });

  expect(getCommit).toHaveBeenCalledWith("4c1a7e2");
  // The role, not the raw text: an addition and a removal are told apart by
  // colour rather than by their leading character alone.
  const styles = tree.root.findAll(node =>
    String(node.type) === "Text" && node.props.style && Array.isArray(node.props.style));
  const colours = styles
    .flatMap(node => (node.props.style as unknown[]).flat())
    .filter((entry): entry is { color?: string } => !!entry && typeof entry === "object")
    .map(entry => entry.color);
  expect(colours).toContain("#success");
  expect(colours).toContain("#danger");
});

test("a row says who made the commit, then its hash, then when", async () => {
  await mount();
  const painted = rowPress("4c1a7e2").findAll(node => String(node.type) === "Text")
    .map(node => node.children.map(String).join(""));
  // Three fields on the second line, in the order a reader looks for them: the
  // author is what identifies a line of work, the hash is what you copy out, the
  // date is when it landed.
  expect(painted).toContain("Lixin Zhou · 4c1a7e2 · 2026-11-11 18:42");
});

test("the commit's dot sits on the subject's line, not in the gap between the two", async () => {
  await mount();
  const circles = rowPress("9b30d18").findAllByType(Circle);
  expect(circles).toHaveLength(1);
  // The row stacks two lines of text, so its own middle is the gap between them.
  // The dot is drawn at the first line's centre instead. With the mock theme's
  // identity `leading`, the row is `max(34, 19 + 11 + 8)` = 38 high and holds
  // 19 + 14 = 33 of text, so that centre is (38 - 33)/2 + 19/2 = 12 — seven
  // points above the row's own middle.
  expect(circles[0]!.props.cy).toBe(12);
});

/** The patch page's header is gone too, so what it used to say moved into the
 * body — and the header's absent state is what this asserts. */
test("the patch page has no header either: the commit says who and when itself", async () => {
  await mount();
  act(() => rowPress("9b30d18").props.onPress());
  await act(async () => { await Promise.resolve(); });

  const painted = texts();
  expect(painted.some(text => text.includes("Lixin Zhou") && text.includes("2026-11-11 16:05")))
    .toBe(true);
  // The same three fields in the same order as the row it was opened from: the
  // patch page is the same commit, so it does not reorder them. The sha is its
  // own selectable `Text` inside the line, so the line is read with its nested
  // children in place rather than node by node.
  const meta = tree.root
    .findAll(node => String(node.type) === "Text" && textOf(node).includes("9b30d18"))
    .map(textOf)
    .find(text => text.split("9b30d18").length === 2 && text.includes("2026-11-11"));
  expect(meta).toBe("Lixin Zhou · 9b30d18 · 2026-11-11 16:05");
  expect(painted).toContain("merge side");
  // The only control is the way back to the list.
  expect(escapeButton("git.back")).toBeDefined();
});

test("the detail pane is the one place a commit's bytes can be read, and back pops it", async () => {
  await mount();
  act(() => rowPress("e27f5a0").props.onPress());
  await act(async () => { await Promise.resolve(); });
  expect(getCommit).toHaveBeenCalledWith("e27f5a0");

  // The back button returns to the list, and the list's back button leaves.
  act(() => escapeButton("git.back").props.onPress());
  expect(onClose).not.toHaveBeenCalled();
  act(() => escapeButton("common.close").props.onPress());
  expect(onClose).toHaveBeenCalledTimes(1);
});

test("system back pops the patch before it closes the sheet", async () => {
  await mount();
  act(() => rowPress("e27f5a0").props.onPress());
  await act(async () => { await Promise.resolve(); });

  // Android's edge gesture leaves the reader on the list they were reading…
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

test("a pull-down re-reads the history, since there is no refresh button", async () => {
  await mount();
  const refresh = commits().props.refreshControl;
  expect(refresh).toBeDefined();
  await act(async () => { await refresh.props.onRefresh(); });
  expect(listCommits).toHaveBeenCalledTimes(2);
});

test("a cut log says the oldest commits are not shown", async () => {
  listCommits.mockResolvedValueOnce({ ...listing, truncated: true });
  await mount();
  expect(texts()).toContain("git.truncated");
});
