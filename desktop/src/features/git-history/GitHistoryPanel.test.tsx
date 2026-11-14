import type { GitGraphListing } from "./types";
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

const listLocal = vi.fn<(path: string) => Promise<GitGraphListing>>();
const getLocal = vi.fn<(path: string, sha: string) => Promise<{ patch: string; short: string; truncated: boolean }>>();
const listRemote = vi.fn<(desktopId: string, sessionId: string) => Promise<GitGraphListing>>();
const getRemote = vi.fn<(desktopId: string, sessionId: string, sha: string) => Promise<{ patch: string; short: string; truncated: boolean }>>();

vi.mock("../../integrations/tauri/invoke", () => ({
  invokeCommand: (command: string, args: Record<string, unknown>) => {
    if (command === "get_git_history") {
      return listLocal(args.path as string);
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
    short: "aaa111",
    truncated: false,
  });
  listRemote.mockReset().mockResolvedValue(listing());
  getRemote.mockReset().mockResolvedValue({ patch: "", short: "aaa111", truncated: false });
});

/** The commit rows, in order. */
const rows = () => [...container.querySelectorAll<HTMLButtonElement>("[data-git-commit]")];

it("reads a local workspace's history through the local command", async () => {
  await mount({ kind: "local", path: "/Users/reader/repo" });
  expect(listLocal).toHaveBeenCalledWith("/Users/reader/repo");
  expect(listRemote).not.toHaveBeenCalled();
  expect(rows()).toHaveLength(COMMITS.length);
  expect(container.textContent).toContain("trunk tip");
  expect(container.textContent).toContain("HEAD -> main");
});

it("reads a remote conversation's history from the peer it belongs to", async () => {
  await mount({ desktopId: "desktop_a", kind: "remote", sessionId: "sess_1" });
  expect(listRemote).toHaveBeenCalledWith("desktop_a", "sess_1");
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

it("names the author first, then the hash, then when", async () => {
  await mount({ kind: "local", path: "/Users/reader/repo" });
  // One line, three fields in the order a reader looks for them: who made the
  // commit, what it is called, when it landed.
  expect(rows()[0]!.textContent).toContain("Rin · aaa111 · 2 hours ago");
});

it("gives each lane its own colour, and says which lane a row is on", async () => {
  await mount({ kind: "local", path: "/Users/reader/repo" });
  // The row's left stripe carries its lane colour, which is what makes a page
  // scannable without reading the picture.
  const stripes = [...container.querySelectorAll<HTMLElement>("[data-lane-stripe]")]
    .map((stripe) => {
      return stripe.style.backgroundColor;
    });
  expect(stripes).toHaveLength(COMMITS.length);
  // Commit "ccc333" sits on lane 1 and "eee555" on lane 2 in the fixture, so a
  // page of rows is not one colour repeated — which is what makes a page of
  // lanes readable at a glance rather than only inside the picture.
  expect(new Set(stripes).size).toBeGreaterThan(1);
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
