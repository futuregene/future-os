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
      accent: "#accent",
      ink: "#ink",
      inkMuted: "#muted",
      inkSoft: "#soft",
      inkStrong: "#strong",
      danger: "#danger",
      dangerSoft: "#dangerSoft",
      success: "#success",
      successSoft: "#successSoft",
      info: "#info",
      infoSoft: "#infoSoft",
      line: "#line",
      lineSoft: "#line",
      surface: "#surface",
      surfaceSubtle: "#subtle",
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
  refs: {
    branches: ["feature/analysis"],
    remotes: ["origin/main"],
    tags: ["v3.2"],
    head: false,
  },
};

let tree: ReactTestRenderer;
const listCommits = jest.fn<Promise<GitGraphListing>, [string?]>(async () => listing);
const getCommit = jest.fn<Promise<GitCommitDetail>, [string]>(async () => detail);
const onClose = jest.fn();
const removeBack = jest.fn();
/** The stepper the panel hands the conversation bar, if it registered one. */
let panelBack: (() => boolean) | null = null;
const registerBack = jest.fn((handler: () => boolean) => { panelBack = handler; });

const mount = async (overrides: Partial<Parameters<typeof GitGraphPanel>[0]> = {}) => {
  await act(async () => {
    tree = create(createElement(GitGraphPanel, {
      online: true, supported: true, listCommits, getCommit, onClose, registerBack,
      ...overrides,
    }));
  });
  await act(async () => { await Promise.resolve(); });
};
const commits = () => tree.root.findByType(FlatList);

test("the first commit is not flush against the surface above it", async () => {
  await mount();
  // The history starts immediately under the conversation's bar, so the list
  // gives it a little air rather than butting the first row against the edge.
  const content = commits().props.contentContainerStyle;
  const style = Object.assign({}, ...[content].flat().filter(
    (entry): entry is Record<string, unknown> => !!entry && typeof entry === "object",
  ));
  expect(style.paddingTop).toBeGreaterThan(0);
});
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
/** Whether any control carries this label right now. */
const hasButton = (label: string) =>
  tree.root.findAll(node =>
    node.props.accessibilityRole === "button"
    && node.props.accessibilityLabel === label
    && node.props.onPress).length > 0;
/** The control carrying a label, for pressing it. */
const button = (label: string) =>
  tree.root.findAll(node =>
    node.props.accessibilityRole === "button"
    && node.props.accessibilityLabel === label
    && node.props.onPress)[0]!;

beforeEach(() => {
  jest.clearAllMocks();
  panelBack = null;
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

/** Neither page has a header bar, and the list has no control of its own. */
test("the list has no header and no second way out", async () => {
  await mount();
  // Nothing names the repository at the top of the list; the commits are the
  // page.
  expect(texts()).not.toContain("dopamine-decision");
  expect(texts()).not.toContain("/Users/lixin/Research/dopamine-decision");
  // And no arrow of its own: the conversation bar's back button is the only back
  // control on screen, and it already leaves the history from here.
  const labels = tree.root
    .findAll(node => node.props.accessibilityRole === "button")
    .map(node => node.props.accessibilityLabel)
    .filter((label): label is string => typeof label === "string");
  expect(labels).not.toContain("common.close");
  expect(labels).not.toContain("git.back");
  // Nothing is left to pop, so the bar's back should be free to leave.
  expect(panelBack?.()).toBe(false);
  expect(onClose).not.toHaveBeenCalled();
});
test("a tapped commit opens its patch, coloured by line role", async () => {
  await mount();
  act(() => rowPress("4c1a7e2").props.onPress());
  await act(async () => { await Promise.resolve(); });

  expect(getCommit).toHaveBeenCalledWith("4c1a7e2");
  // The role, not the raw text: an addition and a removal are told apart by
  // colour rather than by their leading character alone.
  const entryStyles = patchEntries();
  const colours = entryStyles.map(entry => entry.color);
  expect(colours).toContain("#success");
  expect(colours).toContain("#danger");
});

/**
 * The style objects the patch page paints each line with.
 *
 * A line's role is spread over several style entries (the base line, then the
 * role), so the merged view is what a reader actually sees.
 */
function patchEntries() {
  return tree.root
    .findAll(node => String(node.type) === "Text" && Array.isArray(node.props.style))
    .map(node => Object.assign({}, ...(node.props.style as unknown[]).flat().filter(
      (entry): entry is Record<string, unknown> => !!entry && typeof entry === "object",
    )));
}

test("the patch tints each line across its width, with a bar at the left", async () => {
  await mount();
  act(() => rowPress("4c1a7e2").props.onPress());
  await act(async () => { await Promise.resolve(); });

  const entries = patchEntries();
  // A tint plus a bar, not a text hue alone: 11px monospace on a phone screen is
  // too small to sort by hue, and the bar is what a thumb follows down the page.
  const added = entries.find(entry => entry.color === "#success");
  expect(added?.backgroundColor).toBe("#successSoft");
  expect(added?.borderLeftColor).toBe("#success");
  expect(added?.borderLeftWidth).toBe(2);

  const removed = entries.find(entry => entry.color === "#danger");
  expect(removed?.backgroundColor).toBe("#dangerSoft");
  expect(removed?.borderLeftColor).toBe("#danger");

  const hunk = entries.find(entry => entry.color === "#info");
  expect(hunk?.backgroundColor).toBe("#infoSoft");
  expect(hunk?.borderLeftColor).toBe("#info");

  // Context stays untinted and unbarred, so the tinted rows are what stands out.
  const plain = entries.find(entry => entry.color === "#soft" && entry.borderLeftWidth === 2);
  expect(plain?.backgroundColor).toBeUndefined();
  expect(plain?.borderLeftColor).toBe("transparent");
});

test("the tint runs the full width of the row, not just behind the text", async () => {
  await mount();
  act(() => rowPress("4c1a7e2").props.onPress());
  await act(async () => { await Promise.resolve(); });

  // The coloured band belongs to the line, and the line owns its own padding:
  // a padded container would leave an untinted gutter down both edges.
  const card = tree.root.findAll(node =>
    String(node.type) === "View" && (node.props.style as { overflow?: string } | undefined)?.overflow === "hidden");
  expect(card.length).toBeGreaterThan(0);
  const entries = patchEntries();
  const tinted = entries.filter(entry => entry.backgroundColor === "#successSoft" || entry.backgroundColor === "#dangerSoft");
  expect(tinted.length).toBeGreaterThan(0);
  for (const entry of tinted) {
    expect(entry.paddingHorizontal).toBeGreaterThan(0);
  }
});

test("a row says when, then its hash, then who made it", async () => {
  await mount();
  const painted = rowPress("4c1a7e2").findAll(node => String(node.type) === "Text")
    .map(node => node.children.map(String).join(""));
  // Three fields on the second line: the date first ("is this the change I am
  // after?"), then the hash you copy out, then the author.
  expect(painted).toContain("2026-11-11 18:42 · 4c1a7e2 · Lixin Zhou");
});

test("the patch page lists every ref on the commit, one kind per row", async () => {
  await mount();
  act(() => rowPress("4c1a7e2").props.onPress());
  await act(async () => { await Promise.resolve(); });

  const painted = texts();
  // Branches, remotes and tags are separate rows: "is it released?" and "is it
  // pushed?" are different questions, and the list row's one-line decoration
  // cannot answer either once a commit carries several refs.
  for (const name of ["feature/analysis", "origin/main", "v3.2"]) {
    expect(painted).toContain(name);
  }
  expect(painted).toContain("git.ref.branches");
  expect(painted).toContain("git.ref.remotes");
  expect(painted).toContain("git.ref.tags");
});

test("a commit that is the checkout says so, even with no branch on it", async () => {
  getCommit.mockResolvedValueOnce({
    ...detail,
    // A detached checkout of an old commit: no branch and no tag point at it.
    refs: { branches: [], head: true, remotes: [], tags: [] },
  });
  await mount();
  act(() => rowPress("4c1a7e2").props.onPress());
  await act(async () => { await Promise.resolve(); });

  const painted = texts();
  expect(painted).toContain("git.ref.checkedOut");
  expect(painted).toContain("git.ref.yes");
});

test("a commit with no refs shows no refs block at all", async () => {
  getCommit.mockResolvedValueOnce({
    ...detail,
    refs: { branches: [], head: false, remotes: [], tags: [] },
  });
  await mount();
  act(() => rowPress("4c1a7e2").props.onPress());
  await act(async () => { await Promise.resolve(); });

  const painted = texts();
  expect(painted).not.toContain("git.ref.branches");
  expect(painted).not.toContain("git.ref.checkedOut");
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
  expect(meta).toBe("2026-11-11 16:05 · 9b30d18 · Lixin Zhou");
  expect(painted).toContain("merge side");
  // No second arrow on the patch either: the bar's back is the one control, and
  // it steps back to the list before it leaves.
  expect(hasButton("git.back")).toBe(false);
  expect(hasButton("common.close")).toBe(false);
});

test("the detail pane is the one place a commit's bytes can be read, and back pops it", async () => {
  await mount();
  act(() => rowPress("e27f5a0").props.onPress());
  await act(async () => { await Promise.resolve(); });
  expect(getCommit).toHaveBeenCalledWith("e27f5a0");

  // The bar's back steps back to the list, and only then leaves the history.
  act(() => { expect(panelBack?.()).toBe(true); });
  expect(onClose).not.toHaveBeenCalled();
  await act(async () => { await Promise.resolve(); });
  // Back on the list: the commit rows are what is painted again, not the patch.
  expect(commits().props.data).toHaveLength(5);
  expect(texts()).not.toContain("merge side");
  // …and the same press now has nothing to step out of, so it leaves.
  expect(panelBack?.()).toBe(false);
  expect(onClose).not.toHaveBeenCalled();
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

/** The history is longer than one reply: the next page is `listing`'s older
 * half, drawn with the lanes the page above it opened. */
const olderPage: GitGraphListing = {
  ...listing,
  commits: [
    { short: "8f1c002", when: "2026-11-10 21:14", who: "Mei Tan", decorations: "", subject: "early plot", graph: "│ o │ " },
    { short: "0d4b7a9", when: "2026-11-09 10:02", who: "Lixin Zhou", decorations: "tag: v1", subject: "first data", graph: "o─┴─┘ " },
  ],
  truncated: false,
};

test("reaching the end appends the next page instead of stopping there", async () => {
  listCommits.mockResolvedValueOnce({ ...listing, truncated: true, cursor: "at:5" });
  listCommits.mockResolvedValueOnce(olderPage);
  await mount();
  expect(commits().props.data).toHaveLength(5);
  // The first page says the log goes on, so the footer offers the rest.
  expect(hasButton("git.loadMore")).toBe(true);

  await act(async () => { await commits().props.onEndReached!(); });

  // The cursor is where the page on screen stopped: the host reads the next page
  // from it, rather than counting rows the sheet happens to hold.
  expect(listCommits).toHaveBeenLastCalledWith("at:5");
  expect(commits().props.data).toHaveLength(7);
  expect(commits().props.data.at(-1)!.short).toBe("0d4b7a9");
  // The appended rows are drawn with the lanes of the rows above them: a page
  // laid out on its own would start every line again at the left edge.
  expect(rowPress("0d4b7a9").findAllByType(Circle)[0]!.props.fill).not.toBe("none");
  // Nothing is left past this page, and the list says so rather than pretending
  // there is more.
  expect(texts()).toContain("git.historyEnd");
  expect(hasButton("git.loadMore")).toBe(false);
});

test("one scroll asks for one page, however often the list reports its end", async () => {
  listCommits.mockResolvedValueOnce({ ...listing, truncated: true });
  let settle: (page: GitGraphListing) => void = () => {};
  listCommits.mockImplementationOnce(() => new Promise<GitGraphListing>((resolve) => {
    settle = resolve;
  }));
  await mount();
  // `onEndReached` fires again on every re-render near the end of a list; the
  // rows must not be appended twice for one scroll.
  act(() => { void commits().props.onEndReached!(); void commits().props.onEndReached!(); });
  expect(listCommits).toHaveBeenCalledTimes(2);

  await act(async () => { settle(olderPage); });
  expect(commits().props.data).toHaveLength(7);
});

test("a page that fails keeps what was read and offers the retry", async () => {
  listCommits.mockResolvedValueOnce({ ...listing, truncated: true, cursor: "at:5" });
  listCommits.mockRejectedValueOnce(new Error("nope"));
  await mount();
  await act(async () => { await commits().props.onEndReached!(); });

  // The rows already on screen stay: a failed page is not a failed history.
  expect(commits().props.data).toHaveLength(5);
  expect(texts()).toContain("git.loadMoreFailed");

  listCommits.mockResolvedValueOnce(olderPage);
  await act(async () => { await button("git.loadMoreFailed").props.onPress(); });
  // The retry asks from the same place: the page that failed left the cursor
  // where it was, so nothing is skipped and nothing repeats.
  expect(listCommits).toHaveBeenLastCalledWith("at:5");
  expect(commits().props.data).toHaveLength(7);
});

test("a desktop too old to page stops at the page it has, and says so", async () => {
  // An older host answers the second request with the first page again: nothing
  // new arrives, and it still reports that more exists.
  listCommits.mockResolvedValueOnce({ ...listing, truncated: true });
  listCommits.mockResolvedValueOnce({ ...listing, truncated: true });
  await mount();
  await act(async () => { await commits().props.onEndReached!(); });

  expect(listCommits).toHaveBeenCalledTimes(2);
  // No row is drawn twice…
  expect(commits().props.data).toHaveLength(5);
  // …and the button is gone, because pressing it again could not do anything.
  expect(hasButton("git.loadMore")).toBe(false);
  expect(texts()).toContain("git.truncated");
});
