import type { GitCommitDetail, GitGraphListing } from "./types";
// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { beforeEach, expect, it, vi } from "vitest";
import { GitHistoryPanel } from "./GitHistoryPanel";

/**
 * The desktop's Git history page, for one conversation's workspace.
 *
 * Which machine answered is not this component's business — it takes a
 * `GitHistorySource` — so both paths are driven here through the one seam they
 * share. What belongs to the panel is the picture: a row per commit, a lane
 * colour per row, and a click opening the commit's patch. The geometry itself is
 * `@future-os/git-graph`'s, tested there.
 */

const listLocal = vi.fn<(path: string, offset?: number) => Promise<GitGraphListing>>();
const getLocal = vi.fn<(path: string, sha: string) => Promise<GitCommitDetail>>();
const listRemote = vi.fn<(desktopId: string, sessionId: string, offset?: number) => Promise<GitGraphListing>>();
const getRemote = vi.fn<(desktopId: string, sessionId: string, sha: string) => Promise<GitCommitDetail>>();

vi.mock("../../integrations/tauri/invoke", () => ({
  invokeCommand: (command: string, args: Record<string, unknown>) => {
    if (command === "get_git_history") {
      return listLocal(args.path as string, args.offset as number | undefined);
    }
    return getLocal(args.path as string, args.sha as string);
  },
}));

vi.mock("../remote-peer/remotePeerClient", () => ({
  listRemoteGitCommits: (...args: Parameters<typeof listRemote>) => listRemote(...args),
  getRemoteGitCommit: (...args: Parameters<typeof getRemote>) => getRemote(...args),
}));

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

/**
 * A history as the host writes gutters — copied from a real `git log` of this
 * repository, not invented: two worktrees' worth of merges feeding a trunk, so
 * commits sit on lane 0, 1 and 2 and lanes genuinely interleave. Copying real
 * output matters because the decoder reads glyphs, not a shape someone imagined.
 */
const COMMITS = [
  { graph: "o             ", subject: "trunk tip", short: "aaa111", decorations: "HEAD -> main", when: "2 hours ago", who: "Rin" },
  { graph: "M─┐           ", subject: "merge the branch", short: "bbb222", decorations: "", when: "3 hours ago", who: "Rin" },
  { graph: "│ o           ", subject: "work on the branch", short: "ccc333", decorations: "", when: "4 hours ago", who: "Kai" },
  { graph: "o─┘           ", subject: "branch rejoins", short: "ddd444", decorations: "origin/feat", when: "5 hours ago", who: "Kai" },
  { graph: "│ M─┐         ", subject: "second branch merged", short: "eee555", decorations: "", when: "6 hours ago", who: "Jun" },
];

function listing(commits = COMMITS, overrides: Partial<GitGraphListing> = {}): GitGraphListing {
  return {
    commits,
    isRepo: true,
    path: "/Users/reader/repo",
    repoName: "repo",
    truncated: false,
    ...overrides,
  };
}

let container: HTMLDivElement;
let root: ReturnType<typeof createRoot>;

async function flush() {
  await act(async () => {
    await new Promise(resolve => setTimeout(resolve, 0));
  });
}

async function mount(source: { kind: "local"; path: string } | { desktopId: string; kind: "remote"; sessionId: string }) {
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
  await act(async () => {
    root.render(<GitHistoryPanel source={source} />);
  });
  await flush();
}

beforeEach(() => {
  document.body.innerHTML = "";
  listLocal.mockReset().mockResolvedValue(listing());
  getLocal.mockReset().mockResolvedValue({
    patch: "commit aaa111\nAuthor: Rin\n\n    third\n\ndiff --git a/src/a.ts b/src/a.ts\n--- a/src/a.ts\n+++ b/src/a.ts\n@@ -1,2 +1,3 @@\n context\n+added line\n-removed line\n",
    refs: {
      branches: ["main"],
      head: true,
      remotes: ["origin/main"],
      tags: ["v1.0", "v1.1"],
    },
    short: "aaa111",
    truncated: false,
  });
  listRemote.mockReset().mockResolvedValue(listing());
  getRemote.mockReset().mockResolvedValue({
    patch: "",
    refs: { branches: [], head: false, remotes: [], tags: [] },
    short: "aaa111",
    truncated: false,
  });
});

/** The commit rows, in order. */
const rows = () => [...container.querySelectorAll<HTMLButtonElement>("[data-git-commit]")];

it("reads a local workspace's history through the local command", async () => {
  await mount({ kind: "local", path: "/Users/reader/repo" });
  expect(listLocal).toHaveBeenCalledWith("/Users/reader/repo", 0);
  expect(listRemote).not.toHaveBeenCalled();
  expect(rows()).toHaveLength(COMMITS.length);
  expect(container.textContent).toContain("trunk tip");
  expect(container.textContent).toContain("HEAD -> main");
});

it("reads a remote conversation's history from the peer it belongs to", async () => {
  await mount({ desktopId: "desktop_a", kind: "remote", sessionId: "sess_1" });
  expect(listRemote).toHaveBeenCalledWith("desktop_a", "sess_1", 0);
  expect(listLocal).not.toHaveBeenCalled();
  expect(rows()).toHaveLength(COMMITS.length);
});

it("draws lanes as strokes rather than glyphs", async () => {
  await mount({ kind: "local", path: "/Users/reader/repo" });
  // Scoped to the lane pictures: the panel's own toolbar icons are SVG too.
  const svgs = container.querySelectorAll("[data-lane-graph]");
  // One picture per row.
  expect(svgs.length).toBe(COMMITS.length);
  // Every row has at least a commit dot; the box-drawing characters the host
  // sent are not in the DOM at all.
  expect(svgs[0]!.querySelector("circle")).not.toBeNull();
  // The box-drawing characters the host sent are in no text node: they were a
  // description of the picture, and the picture is strokes. (`o` is not checked
  // — it is an ordinary letter, and this fixture's subjects contain it.)
  for (const glyph of ["│", "─", "┴", "┘", "┐", "├", "┤", "┬"]) {
    expect(container.textContent).not.toContain(glyph);
  }
});

it("puts the commit's dot on the subject's line, not in the gap between the lines", async () => {
  await mount({ kind: "local", path: "/Users/reader/repo" });
  const dot = rows()[0]!.querySelector("circle")!;
  // The row stacks a subject and a metadata line, so the row's middle is the gap
  // between them: a dot there sits beside neither. It belongs on the first line,
  // which with a 17px subject line and a 15px metadata line in a 34px row puts
  // its centre at (34 - 32)/2 + 8.5 = 9.5.
  expect(dot.getAttribute("cy")).toBe("9.5");
  // A connection runs on the dot's own axis, so a merge's elbow leaves the dot's
  // height rather than the row's middle.
  const elbow = rows()[1]!.querySelector("path")!.getAttribute("d")!;
  expect(elbow).toContain(" 9.5 ");
});

it("says when, then the hash, then who made it", async () => {
  await mount({ kind: "local", path: "/Users/reader/repo" });
  // One line, three fields: the date first ("is this the change I am after?"),
  // then the hash you copy out, then the author.
  expect(rows()[0]!.textContent).toContain("2 hours ago · aaa111 · Rin");
});

it("lists every ref on the open commit, one kind per row", async () => {
  await mount({ kind: "local", path: "/Users/reader/repo" });
  await act(async () => {
    rows()[0]!.click();
  });
  await flush();

  // The row's decoration is one line and drops local branches once a commit
  // carries several; the detail asks for the whole set instead.
  const refs = container.querySelector("[data-git-refs]");
  expect(refs).not.toBeNull();
  const text = refs!.textContent ?? "";
  for (const name of ["main", "origin/main", "v1.0", "v1.1"]) {
    expect(text).toContain(name);
  }
  expect(text).toContain("Branches");
  expect(text).toContain("Remote");
  expect(text).toContain("Tags");
  // HEAD points at this commit, so the checkout is named too.
  expect(text).toContain("Here");
});

it("draws no refs block for a commit no ref points at", async () => {
  getLocal.mockResolvedValueOnce({
    patch: "commit aaa111\n",
    refs: { branches: [], head: false, remotes: [], tags: [] },
    short: "aaa111",
    truncated: false,
  });
  await mount({ kind: "local", path: "/Users/reader/repo" });
  await act(async () => {
    rows()[0]!.click();
  });
  await flush();

  expect(container.querySelector("[data-git-refs]")).toBeNull();
});

it("says which lane a row is on in words, not with a tinted edge", async () => {
  await mount({ kind: "local", path: "/Users/reader/repo" });
  // The picture is what says which column a commit is on. A hairline of the
  // lane's colour used to sit at the row's left edge; pinned to the row rather
  // than to the lane column, it read as a stray border beside the graph.
  expect(container.querySelectorAll("[data-lane-stripe]")).toHaveLength(0);
  // The lane is still there to be read — in the picture, where the dot's x is
  // the lane column.
  const lanes = [...container.querySelectorAll("[data-lane-graph] circle")]
    .map((dot) => {
      return dot.getAttribute("cx");
    });
  expect(new Set(lanes).size).toBeGreaterThan(1);
});

it("opens a commit's patch, coloured by line role", async () => {
  await mount({ kind: "local", path: "/Users/reader/repo" });
  await act(async () => {
    rows()[0]!.click();
  });
  await flush();

  expect(getLocal).toHaveBeenCalledWith("/Users/reader/repo", "aaa111");
  expect(container.textContent).toContain("added line");
  expect(container.textContent).toContain("removed line");

  const added = [...container.querySelectorAll("[data-patch-role]")]
    .find(node => node.getAttribute("data-patch-role") === "added");
  const removed = [...container.querySelectorAll("[data-patch-role]")]
    .find(node => node.getAttribute("data-patch-role") === "removed");
  expect(added?.className).toContain("text-success");
  expect(removed?.className).toContain("text-danger");
  // A tinted band and a bar at the left, not a text hue alone — the same
  // reading the phone gives a patch, so one patch looks the same on both.
  expect(added?.className).toContain("bg-success-soft");
  expect(added?.className).toContain("border-success");
  expect(removed?.className).toContain("bg-danger-soft");
  expect(removed?.className).toContain("border-danger");

  // Context is neither tinted nor barred, so the tinted rows are what stands
  // out. The bar is kept (transparent) so every line indents by the same
  // amount and the patch columns stay aligned.
  const context = [...container.querySelectorAll("[data-patch-role]")]
    .find(node => node.getAttribute("data-patch-role") === "plain");
  expect(context?.className).toContain("border-transparent");
  expect(context?.className).not.toContain("bg-success-soft");
  expect(context?.className).not.toContain("bg-danger-soft");
});

it("reads a remote commit's patch from the peer, not from this machine", async () => {
  await mount({ desktopId: "desktop_a", kind: "remote", sessionId: "sess_1" });
  await act(async () => {
    rows()[1]!.click();
  });
  await flush();
  expect(getRemote).toHaveBeenCalledWith("desktop_a", "sess_1", "bbb222");
  expect(getLocal).not.toHaveBeenCalled();
});

it("goes back from the patch to the list", async () => {
  await mount({ kind: "local", path: "/Users/reader/repo" });
  await act(async () => {
    rows()[0]!.click();
  });
  await flush();
  expect(rows()).toHaveLength(0);

  const back = container.querySelector<HTMLButtonElement>("[data-git-back]");
  await act(async () => {
    back!.click();
  });
  await flush();
  expect(rows()).toHaveLength(COMMITS.length);
});

it("says so instead of offering rows when the workspace is not a repository", async () => {
  listLocal.mockResolvedValue(listing([], { isRepo: false, repoName: "" }));
  await mount({ kind: "local", path: "/Users/reader/notes" });
  expect(rows()).toHaveLength(0);
  expect(container.textContent).toContain("not a Git repository");
});

it("shows a failure rather than an empty history", async () => {
  listLocal.mockRejectedValue(new Error("git not found"));
  await mount({ kind: "local", path: "/Users/reader/repo" });
  expect(container.textContent).toContain("git not found");
  expect(rows()).toHaveLength(0);
});

it("appends the next page instead of stopping at the first one", async () => {
  listLocal.mockResolvedValueOnce(listing(COMMITS, { truncated: true }));
  listLocal.mockResolvedValueOnce(listing([
    { decorations: "", graph: "│ o  ", short: "fff666", subject: "older work", when: "7 hours ago", who: "Jun" },
  ]));
  await mount({ kind: "local", path: "/Users/reader/repo" });
  expect(rows()).toHaveLength(COMMITS.length);

  const more = container.querySelector<HTMLButtonElement>("[data-git-load-more]")!;
  expect(more.textContent).toContain("Load older commits");
  await act(async () => {
    more.click();
  });
  await flush();

  // The offset is what the view already holds: one log, read a page at a time.
  expect(listLocal).toHaveBeenLastCalledWith("/Users/reader/repo", COMMITS.length);
  expect(rows()).toHaveLength(COMMITS.length + 1);
  expect(rows()[COMMITS.length]!.textContent).toContain("older work");
  // Nothing is left past this page, and the panel says so rather than leaving
  // the reader looking at a button that would return nothing.
  expect(container.querySelector("[data-git-load-more]")).toBeNull();
  expect(container.textContent).toContain("This is the oldest commit.");
});

it("keeps the rows already read when a page fails, and offers the retry", async () => {
  listLocal.mockResolvedValueOnce(listing(COMMITS, { truncated: true }));
  listLocal.mockRejectedValueOnce(new Error("git timed out"));
  await mount({ kind: "local", path: "/Users/reader/repo" });

  await act(async () => {
    container.querySelector<HTMLButtonElement>("[data-git-load-more]")!.click();
  });
  await flush();

  expect(rows()).toHaveLength(COMMITS.length);
  const more = container.querySelector<HTMLButtonElement>("[data-git-load-more]")!;
  expect(more.textContent).toContain("Could not read older commits");

  listLocal.mockResolvedValueOnce(listing([
    { decorations: "", graph: "│ o  ", short: "fff666", subject: "older work", when: "7 hours ago", who: "Jun" },
  ]));
  await act(async () => {
    more.click();
  });
  await flush();
  expect(listLocal).toHaveBeenLastCalledWith("/Users/reader/repo", COMMITS.length);
  expect(rows()).toHaveLength(COMMITS.length + 1);
});

it("stops at the page it has when the peer cannot page, and says so", async () => {
  // A peer that predates paging answers the second request with the first page
  // again: nothing new arrives, and it still reports that more exists.
  listRemote.mockResolvedValueOnce(listing(COMMITS, { truncated: true }));
  listRemote.mockResolvedValueOnce(listing(COMMITS, { truncated: true }));
  await mount({ desktopId: "desktop_a", kind: "remote", sessionId: "sess_1" });

  await act(async () => {
    container.querySelector<HTMLButtonElement>("[data-git-load-more]")!.click();
  });
  await flush();

  expect(listRemote).toHaveBeenCalledTimes(2);
  // No row is drawn twice…
  expect(rows()).toHaveLength(COMMITS.length);
  // …and the button is gone, because pressing it again could not do anything.
  expect(container.querySelector("[data-git-load-more]")).toBeNull();
  expect(container.textContent).toContain("The oldest commits are not shown.");
});
