import type { RecentItem } from "./recentOrder";
import { describe, expect, it } from "vitest";
import {
  byActivity,
  byCreation,
  expandRecent,
  isActiveRun,
  RECENT_PAGE_SIZE,

  recentSplit,
  recentTree,
} from "./recentOrder";

const minute = 60_000;
const hour = 60 * minute;
const now = Date.UTC(2026, 9, 21, 12, 0, 0);

function item(key: string, extra: Partial<RecentItem> = {}): RecentItem {
  return {
    key,
    pinned: false,
    active: false,
    createdAt: now - 24 * hour,
    lastActivityAt: now,
    ...extra,
  };
}

const keys = (rows: { key: string }[]) => rows.map(row => row.key);
function sectionKinds(split: ReturnType<typeof recentSplit<RecentItem>>) {
  return split.sections.map(section => section.kind);
}
function countOf(split: ReturnType<typeof recentSplit<RecentItem>>, kind: "active" | "earlier") {
  return split.sections.find(section => section.kind === kind)?.all.length;
}

describe("what counts as still running", () => {
  it("is a run producing output, queued, or waiting on the reader", () => {
    expect(isActiveRun("running")).toBe(true);
    expect(isActiveRun("queued")).toBe(true);
    // The one that matters most: a run blocked on a question is *not* streaming,
    // and filing it under "earlier" is how the reader has to hunt for it.
    expect(isActiveRun("waiting_approval")).toBe(true);
    expect(isActiveRun("completed")).toBe(false);
    expect(isActiveRun("failed")).toBe(false);
    expect(isActiveRun(undefined)).toBe(false);
  });

  it("follows a run another client started, which only reports streaming", () => {
    // A prompt started by the phone or the TUI: the local run row may be absent
    // or already finished, and the session still says it is streaming.
    expect(isActiveRun(undefined, true)).toBe(true);
    expect(isActiveRun("completed", true)).toBe(true);
    expect(isActiveRun("completed", false)).toBe(false);
  });
});

describe("the recent view's order", () => {
  it("ranks active rows by when they started, not by their last reply", () => {
    // A run rewrites its activity stamp about once a second, so ranking by that
    // swaps rows under the reader's thumb. The start time does not move.
    const startedFirstSpeakingLast = item("old-start", {
      active: true,
      createdAt: now - 3 * hour,
      lastActivityAt: now - 1_000,
    });
    const startedLast = item("new-start", {
      active: true,
      createdAt: now - minute,
      lastActivityAt: now - minute,
    });
    expect(keys([startedFirstSpeakingLast, startedLast].sort(byCreation)))
      .toEqual(["new-start", "old-start"]);
    // And by activity it would be the other way round — which is the jump this
    // ordering exists to remove.
    expect(keys([startedFirstSpeakingLast, startedLast].sort(byActivity)))
      .toEqual(["old-start", "new-start"]);
  });

  it("ranks the earlier stretch by the last reply even when a start time exists", () => {
    const touchedLast = item("started-first", { createdAt: now - 5 * hour, lastActivityAt: now - minute });
    const startedLast = item("touched-first", { createdAt: now - minute, lastActivityAt: now - hour });
    expect(keys([startedLast, touchedLast].sort(byActivity)))
      .toEqual(["started-first", "touched-first"]);
  });

  it("puts a row with no stamp below every dated row, and never guesses", () => {
    expect(keys([item("undated", { lastActivityAt: null }), item("dated", { lastActivityAt: now - hour })].sort(byActivity)))
      .toEqual(["dated", "undated"]);
    // A source with no start time falls back to its activity stamp for the
    // active stretch rather than claiming to have started just now.
    expect(keys([item("no-start", { active: true, createdAt: null, lastActivityAt: now - 2 * hour }), item("stamped", { active: true, createdAt: now - hour })].sort(byCreation)))
      .toEqual(["stamped", "no-start"]);
  });

  it("breaks ties on the row key, so a poll cannot reshuffle the list", () => {
    const a = item("a", { lastActivityAt: now });
    const b = item("b", { lastActivityAt: now });
    expect(keys([b, a].sort(byActivity))).toEqual(["a", "b"]);
  });
});

describe("the recent view's hierarchy", () => {
  it("ranks a conversation, not the children that do its work", () => {
    // The `/future-goal` shape: a conversation that started a worker session per
    // piece of work. The worker is newer than its parent by construction, so a
    // list that ranked every session put the worker above the conversation the
    // reader actually made.
    const tree = recentTree([
      item("parent", { createdAt: now - 2 * hour }),
      item("worker", { createdAt: now - minute, parentKey: "parent" }),
      item("other", { createdAt: now - hour }),
    ]);
    expect(keys(tree.roots)).toEqual(["parent", "other"]);
    expect(keys(tree.children.get("parent") ?? [])).toEqual(["worker"]);
  });

  it("keeps a pinned child a child: this view has no pinned section to hoist it into", () => {
    const tree = recentTree([
      item("parent"),
      item("pinned-worker", { parentKey: "parent", pinned: true }),
    ]);
    expect(keys(tree.roots)).toEqual(["parent"]);
  });

  it("makes a child whose parent is not in the list a root", () => {
    // The filter, a search or a deleted parent: a row that cannot be drawn under
    // something is drawn on its own rather than lost.
    const tree = recentTree([item("orphan", { parentKey: "missing" })]);
    expect(keys(tree.roots)).toEqual(["orphan"]);
    expect(tree.children.size).toBe(0);
  });

  it("breaks a malformed ring instead of losing every row in it", () => {
    const tree = recentTree([
      item("a", { parentKey: "b" }),
      item("b", { parentKey: "a" }),
    ]);
    expect(tree.roots).toHaveLength(1);
    expect(keys(expandRecent(tree.roots, tree, new Set(tree.roots.map(row => row.key))).rows)).toHaveLength(2);
  });

  it("draws a child only inside its parent's fold", () => {
    const tree = recentTree([
      item("parent"),
      item("worker", { parentKey: "parent" }),
    ]);
    const folded = expandRecent(tree.roots, tree, new Set());
    expect(keys(folded.rows)).toEqual(["parent"]);
    expect(folded.meta.get("parent")).toEqual({ depth: 0, hasChildren: true });

    const opened = expandRecent(tree.roots, tree, new Set(["parent"]));
    expect(keys(opened.rows)).toEqual(["parent", "worker"]);
    expect(opened.meta.get("worker")).toEqual({ depth: 1, hasChildren: false });
    // The keyboard's block: a child belongs to the conversation it explains, so
    // stepping by conversation does not stop on it separately.
    expect(opened.roots.get("worker")).toBe("parent");
  });

  it("draws a grandchild only when its own parent is open", () => {
    const tree = recentTree([
      item("parent"),
      item("worker", { parentKey: "parent" }),
      item("helper", { parentKey: "worker" }),
    ]);
    expect(keys(expandRecent(tree.roots, tree, new Set(["parent"])).rows)).toEqual(["parent", "worker"]);
    expect(keys(expandRecent(tree.roots, tree, new Set(["parent", "worker"])).rows))
      .toEqual(["parent", "worker", "helper"]);
  });

  it("keeps indenting at three levels, whatever the chain's length", () => {
    const tree = recentTree([
      item("a"),
      item("b", { parentKey: "a" }),
      item("c", { parentKey: "b" }),
      item("d", { parentKey: "c" }),
    ]);
    const expanded = expandRecent(tree.roots, tree, new Set(["a", "b", "c"]));
    expect(keys(expanded.rows)).toEqual(["a", "b", "c", "d"]);
    expect(expanded.meta.get("d")?.depth).toBe(2);
  });
});

describe("the recent view's split", () => {
  it("puts the running rows above the rest, ranked within each stretch", () => {
    const split = recentSplit([
      item("idle-new", { lastActivityAt: now - minute }),
      item("busy-old", { active: true, createdAt: now - 10 * minute }),
      item("idle-old", { lastActivityAt: now - hour }),
      item("busy-new", { active: true, createdAt: now - 2 * minute }),
    ]);
    expect(sectionKinds(split)).toEqual(["active", "earlier"]);
    expect(keys(split.rows)).toEqual(["busy-new", "busy-old", "idle-new", "idle-old"]);
    // Each caption counts its whole stretch.
    expect(countOf(split, "active")).toBe(2);
    expect(countOf(split, "earlier")).toBe(2);
  });

  it("ignores pinning for the order — a pin is a marker here, not a place", () => {
    const split = recentSplit([
      item("pinned-stale", { pinned: true, lastActivityAt: now - 90 * minute }),
      item("fresh", { lastActivityAt: now - minute }),
    ]);
    expect(keys(split.rows)).toEqual(["fresh", "pinned-stale"]);
  });

  it("draws no caption over an empty stretch", () => {
    // A lone "Active" heading over nothing reads as a list that failed to load.
    expect(sectionKinds(recentSplit([item("idle")]))).toEqual(["earlier"]);
    expect(sectionKinds(recentSplit([item("busy", { active: true })]))).toEqual(["active"]);
    expect(recentSplit([]).rows).toEqual([]);
  });

  it("windows the finished stretch and says how much it holds back", () => {
    const many = Array.from({ length: 60 }, (_, index) =>
      item(`idle-${index}`, { lastActivityAt: now - (index + 1) * minute }));
    const split = recentSplit([item("busy", { active: true, createdAt: now }), ...many]);
    expect(split.rows).toHaveLength(RECENT_PAGE_SIZE + 1);
    // The newest of the window, not an arbitrary slice.
    expect(split.rows[0]!.key).toBe("busy");
    expect(split.rows[1]!.key).toBe("idle-0");
    expect(split.rows[split.rows.length - 1]!.key).toBe(`idle-${RECENT_PAGE_SIZE - 1}`);
    expect(split.more).toBe(60 - RECENT_PAGE_SIZE);
    // The caption keeps counting the whole stretch: it is under a window, not
    // over a short list.
    expect(countOf(split, "earlier")).toBe(60);
  });

  it("never windows what is still running, and opens the window on request", () => {
    const busy = Array.from({ length: 40 }, (_, index) =>
      item(`busy-${index}`, { active: true, createdAt: now - index * minute }));
    expect(recentSplit(busy, 5).rows).toHaveLength(40);

    const many = Array.from({ length: 40 }, (_, index) =>
      item(`idle-${index}`, { lastActivityAt: now - (index + 1) * minute }));
    expect(recentSplit(many, 10).rows).toHaveLength(10);
    expect(recentSplit(many, 10).more).toBe(30);
    expect(recentSplit(many, 40).more).toBe(0);
    // A window that has not opened draws no caption over nothing, but the footer
    // still has the whole stretch to offer: the view shows the offer alone.
    expect(sectionKinds(recentSplit(many, 0))).toEqual([]);
    expect(recentSplit(many, 0).more).toBe(40);
  });
});
