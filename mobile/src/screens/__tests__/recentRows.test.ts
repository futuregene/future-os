import type { RemoteSession } from "../../remote/types";
import type { MergedSessionRow } from "../../remote/mergeSessions";
import { isActiveSession, RECENT_PAGE_SIZE, recentRows } from "../recentRows";

const minute = 60_000;
const now = Date.UTC(2026, 9, 21, 12, 0, 0);

const hour = 60 * minute;
const row = (
  id: string,
  extra: Partial<RemoteSession> = {},
  lastMessageAt: number | null = now,
): MergedSessionRow => ({
  key: id,
  desktopId: "desk-a",
  session: { sessionId: id, threadId: `t-${id}`, title: id, streaming: false, ...extra },
  lastMessageAt,
});

type Page = ReturnType<typeof recentRows>;
const ids = (page: Page) =>
  page.rows.filter(item => item.kind === "session").map(item => (item.kind === "session" ? item.key : ""));
const captions = (page: Page) =>
  page.rows.filter(item => item.kind === "section").map(item => (item.kind === "section" ? item.titleKey : ""));
const countOf = (page: Page, key: string) =>
  page.rows.find(item => item.kind === "section" && item.key.endsWith(key)) as
    | { kind: "section"; count: number }
    | undefined;

describe("what counts as active", () => {
  it("is a run producing output, queued, or waiting on the reader", () => {
    expect(isActiveSession(row("a", { streaming: true }))).toBe(true);
    expect(isActiveSession(row("b", { status: "running" }))).toBe(true);
    expect(isActiveSession(row("c", { status: "queued" }))).toBe(true);
    // The one that matters most: a run blocked on a question is *not* streaming,
    // and filing it under "earlier" is how the reader has to hunt for it.
    expect(isActiveSession(row("d", { status: "waiting_approval" }))).toBe(true);
    expect(isActiveSession(row("e", { status: "completed" }))).toBe(false);
    expect(isActiveSession(row("f", { status: "failed" }))).toBe(false);
    expect(isActiveSession(row("g"))).toBe(false);
  });

  it("follows a run another device started, which only reports streaming", () => {
    // `effectiveRunStatus` is the same rule the row's own indicator reads.
    expect(isActiveSession(row("a", { streaming: true, status: "completed" }))).toBe(true);
  });
});

describe("the recency page", () => {
  it("puts active conversations above the rest, newest first in both", () => {
    const page = recentRows([
      row("idle-new", {}, now - minute),
      row("active-old", { streaming: true }, now - 10 * minute),
      row("idle-old", {}, now - 60 * minute),
      row("active-new", { status: "waiting_approval" }, now - 2 * minute),
    ], "");
    expect(captions(page)).toEqual(["sessions.activeSection", "sessions.earlierSection"]);
    expect(ids(page)).toEqual(["active-new", "active-old", "idle-new", "idle-old"]);
    // The captions carry what is under them, so a reader can see how much moves.
    expect(countOf(page, "activeSection")!.count).toBe(2);
    expect(countOf(page, "earlierSection")!.count).toBe(2);
  });

  it("ranks active rows by when they started, not by their last reply", () => {
    // A run rewrites its last-reply stamp about once a second, so ranking by it
    // swapped rows under the reader's thumb. The start time does not move.
    const page = recentRows([
      // Started first, but the newest reply: with recency order this row would
      // lead; by creation it is last of the three.
      row("busy-old", {
        streaming: true,
        createdAt: now - 3 * hour,
        lastMessageAt: now - 1_000,
      } as Partial<RemoteSession>),
      row("busy-middle", {
        streaming: true,
        createdAt: now - 2 * hour,
        lastMessageAt: now - 2 * hour,
      } as Partial<RemoteSession>),
      row("busy-new", {
        status: "running",
        createdAt: now - minute,
        lastMessageAt: now - minute,
      } as Partial<RemoteSession>),
    ], "");
    expect(ids(page)).toEqual(["busy-new", "busy-middle", "busy-old"]);
  });

  it("does not reorder active rows when only their activity moves", () => {
    const started = [
      row("older-start", { streaming: true, createdAt: now - 3 * hour } as Partial<RemoteSession>),
      row("newer-start", { streaming: true, createdAt: now - 2 * hour } as Partial<RemoteSession>),
    ];
    // Both stretches are asserted explicitly: the order is by start, so the row
    // that started later leads even when the other one has spoken most recently.
    expect(ids(recentRows(started, ""))).toEqual(["newer-start", "older-start"]);
    const after = ids(recentRows([
      { ...started[0]!, lastMessageAt: now - 500 },
      { ...started[1]!, lastMessageAt: now - hour },
    ], ""));
    expect(after).toEqual(["newer-start", "older-start"]);
  });

  it("falls back to recency for a desktop that sends no start time", () => {
    // Older desktops omit the field; recency is the only key they offer, and the
    // page must still draw the stretch in some order.
    const page = recentRows([
      row("quiet", { streaming: true, lastMessageAt: now - 5 * minute } as Partial<RemoteSession>),
      row("chatty", { streaming: true, lastMessageAt: now - minute } as Partial<RemoteSession>),
    ], "");
    expect(ids(page)).toEqual(["chatty", "quiet"]);
  });

  it("orders the earlier stretch by the last reply even when a start time exists", () => {
    // The two stretches use different clocks on purpose: "earlier" answers
    // "what did I last touch", so it stays on the reply stamp.
    const page = recentRows([
      row("started-first-touched-last", {
        createdAt: now - 5 * hour,
        lastMessageAt: now - minute,
      } as Partial<RemoteSession>),
      row("started-last-touched-first", {
        createdAt: now - minute,
        lastMessageAt: now - hour,
      } as Partial<RemoteSession>),
    ], "");
    expect(ids(page)).toEqual(["started-first-touched-last", "started-last-touched-first"]);
  });

  it("ignores pinning for the order, and keeps the marker on the row", () => {
    const rows = recentRows([
      row("pinned-stale", { pinned: true }, now - 90 * minute),
      row("fresh", {}, now - minute),
    ], "");
    expect(ids(rows)).toEqual(["fresh", "pinned-stale"]);
  });

  it("orders rows of several machines by the same one rule", () => {
    const rows = recentRows([
      { ...row("a1", {}, now - 5 * minute), key: "desk-a::a1" },
      { ...row("b1", {}, now - minute), key: "desk-b::b1", desktopId: "desk-b" },
    ], "");
    expect(ids(rows)).toEqual(["desk-b::b1", "desk-a::a1"]);
  });

  it("puts a conversation with no stamp below every dated one", () => {
    const rows = recentRows([row("undated", {}, null), row("dated", {}, now - 10 * minute)], "");
    expect(ids(rows)).toEqual(["dated", "undated"]);
  });

  it("draws no caption over an empty stretch", () => {
    // Every conversation finished: a lone "Active" heading would read as a list
    // that failed to load its rows.
    const rows = recentRows([row("idle", {}, now)], "");
    expect(captions(rows)).toEqual(["sessions.earlierSection"]);
    const active = recentRows([row("busy", { streaming: true }, now)], "");
    expect(captions(active)).toEqual(["sessions.activeSection"]);
    expect(recentRows([], "").rows).toEqual([]);
  });

  it("flattens the page while searching, and finds by workspace name too", () => {
    const rows = recentRows([
      row("busy-plan", { streaming: true, title: "Plan", workspaceName: "Review" }, now),
      row("idle-plan", { title: "Plan B", workspaceName: "Other" }, now - minute),
    ], "review");
    // No captions in a search: a filtered list has no order for them to caption.
    expect(captions(rows)).toEqual([]);
    expect(ids(rows)).toEqual(["busy-plan"]);
  });

  it("windows the finished stretch and says how much it holds back", () => {
    // Sixty finished conversations and one in flight: the page draws a window of
    // the finished ones, because the point of the page is the last few things.
    const many = Array.from({ length: 60 }, (_, index) =>
      row(`idle-${index}`, {}, now - (index + 1) * minute));
    const page = recentRows([row("busy", { streaming: true }, now), ...many], "");
    const earlier = ids(page).filter(id => id !== "busy");
    expect(earlier).toHaveLength(RECENT_PAGE_SIZE);
    // Newest first, so the window holds the newest — not an arbitrary slice.
    expect(earlier[0]).toBe("idle-0");
    expect(earlier.at(-1)).toBe(`idle-${RECENT_PAGE_SIZE - 1}`);
    expect(page.more).toBe(60 - RECENT_PAGE_SIZE);
    // The caption keeps counting the whole stretch: it is under a window, not
    // over a short list.
    expect(countOf(page, "earlierSection")!.count).toBe(60);
  });

  it("never windows what is still in flight", () => {
    const busy = Array.from({ length: 30 }, (_, index) =>
      row(`busy-${index}`, { streaming: true }, now - index * minute));
    const page = recentRows(busy, "", 5);
    expect(ids(page)).toHaveLength(30);
    expect(page.more).toBe(0);
  });

  it("opens the window on request, and draws no caption for a window at zero", () => {
    const many = Array.from({ length: 30 }, (_, index) =>
      row(`idle-${index}`, {}, now - (index + 1) * minute));
    expect(ids(recentRows(many, "", 25))).toHaveLength(25);
    expect(recentRows(many, "", 25).more).toBe(5);
    expect(ids(recentRows(many, "", 30))).toHaveLength(30);
    expect(recentRows(many, "", 30).more).toBe(0);
    // A window that has not opened draws no caption over nothing, but the footer
    // still has the whole stretch to offer.
    expect(captions(recentRows(many, "", 0))).toEqual([]);
    expect(recentRows(many, "", 0).more).toBe(30);
  });

  it("searches past the window, not inside it", () => {
    // The thing being looked for is the oldest conversation of sixty — a result
    // list that said "no matches" while holding it would be wrong.
    const many = [
      ...Array.from({ length: 60 }, (_, index) => row(`idle-${index}`, {}, now - (index + 1) * minute)),
      row("needle", { title: "Needle in the haystack" }, now - 600 * minute),
    ];
    const page = recentRows(many, "needle", RECENT_PAGE_SIZE);
    expect(ids(page)).toEqual(["needle"]);
    expect(page.more).toBe(0);
  });

});

describe("child sessions on the recency page", () => {
  /**
   * The `/future-goal` shape: a conversation that started a worker session for
   * each piece of work. The workers are newer than the conversation, so a page
   * that ranked every session put them above it and buried the conversations the
   * reader actually made.
   */
  const goal = (): MergedSessionRow[] => [
    row("goal", {}, now - 2 * minute),
    row("worker", { parentSessionId: "goal" }, now - minute),
    row("helper", { parentSessionId: "worker" }, now - 30_000),
    row("other", {}, now - 10 * minute),
  ];

  it("lists the conversation, not the sessions it started", () => {
    const page = recentRows(goal(), "");
    // By last reply: the conversation spoke more recently than the other row.
    expect(ids(page)).toEqual(["goal", "other"]);
    // The captions count conversations, not the sessions under them.
    expect(countOf(page, "earlierSection")!.count).toBe(2);
  });

  it("keeps a running worker out of the active stretch", () => {
    // A child is never a row of the page's own: filing a goal's worker under
    // "Active" would show a session the page is supposed to leave to its
    // conversation.
    const rows = [
      row("goal", {}, now - 2 * minute),
      row("worker", { parentSessionId: "goal", streaming: true }, now - minute),
    ];
    expect(captions(recentRows(rows, ""))).toEqual(["sessions.earlierSection"]);
  });

  it("draws the children under their conversation when it is opened", () => {
    const first = recentRows(goal(), "");
    const parent = first.rows.find(item => item.kind === "session" && item.key === "goal");
    expect(parent).toMatchObject({ depth: 0, hasChildren: true, foldId: "goal" });

    // Folded: the workers are absent, and the childless row has no toggle.
    expect(ids(first)).toEqual(["goal", "other"]);
    const other = first.rows.find(item => item.kind === "session" && item.key === "other");
    expect(other).toMatchObject({ hasChildren: false });

    // Opened: the direct worker appears, one level in, at its parent's place.
    const opened = recentRows(goal(), "", RECENT_PAGE_SIZE, new Set(["goal"]));
    expect(ids(opened)).toEqual(["goal", "worker", "other"]);
    expect(opened.rows.find(item => item.kind === "session" && item.key === "worker"))
      .toMatchObject({ depth: 1, hasChildren: true, section: "earlier" });

    // Its own children need its own fold, and the indent saturates at three.
    const deep = recentRows(goal(), "", RECENT_PAGE_SIZE, new Set(["goal", "worker"]));
    expect(ids(deep)).toEqual(["goal", "worker", "helper", "other"]);
    expect(deep.rows.find(item => item.kind === "session" && item.key === "helper"))
      .toMatchObject({ depth: 2 });
  });

  it("keeps a pinned child a child: this page has no pinned section to hoist it into", () => {
    const page = recentRows([
      row("parent", {}, now - minute),
      row("pinned-worker", { parentSessionId: "parent", pinned: true }, now - 30 * minute),
    ], "");
    expect(ids(page)).toEqual(["parent"]);
  });

  it("draws a child whose parent is not on the page at the top level", () => {
    // The filter, a search, or a parent that is gone: a row that cannot be drawn
    // under something is drawn on its own rather than lost.
    expect(ids(recentRows([row("orphan", { parentSessionId: "missing" })], ""))).toEqual(["orphan"]);
  });

  it("finds a child by search even when its conversation does not match", () => {
    const rows = [
      row("goal", { title: "Goal" }, now - minute),
      row("worker", { title: "Needle", parentSessionId: "goal" }, now - 30_000),
    ];
    const page = recentRows(rows, "needle");
    expect(ids(page)).toEqual(["worker"]);
    // A search is flat, as on the other two pages.
    expect(page.rows.find(item => item.kind === "session")).toMatchObject({ depth: 0, hasChildren: false });
  });

  it("folds a conversation on another machine by its own key", () => {
    // A session id is that machine's, not global: two peers can each hold `s1`,
    // so a peer's fold is keyed by the machine too.
    const peer = { ...row("s1", {}, now - minute), key: "desk-b::s1", desktopId: "desk-b" };
    const peerWorker = {
      ...row("s2", { parentSessionId: "s1" }, now - 30_000),
      key: "desk-b::s2",
      desktopId: "desk-b",
    };
    expect(ids(recentRows([peer, peerWorker], "", RECENT_PAGE_SIZE, new Set(["s1"]), "desk-a")))
      .toEqual(["desk-b::s1"]);
    expect(ids(recentRows([peer, peerWorker], "", RECENT_PAGE_SIZE, new Set(["desk-b::s1"]), "desk-a")))
      .toEqual(["desk-b::s1", "desk-b::s2"]);
  });

  it("breaks a ring of parents instead of losing every session in it", () => {
    // Two sessions that each name the other as their parent — what a fork made
    // from a fork can look like after a session is re-parented. Following the
    // chain would walk the ring forever, so one edge is cut and the other
    // session keeps its child: both rows still appear, nested.
    const a = row("a", { parentSessionId: "b" }, now);
    const b = row("b", { parentSessionId: "a" }, now - minute);
    // Opened, so the child is drawn: a ring that lost a row would be invisible
    // on a folded page and would silently drop the session entirely.
    const page = recentRows([a, b], "", RECENT_PAGE_SIZE, new Set(["b"]), "desk-a");
    const sessions = page.rows.filter(item => item.kind === "session");
    expect(sessions.map(item => (item.kind === "session" ? item.key : ""))).toHaveLength(2);
    expect(sessions.find(item => item.kind === "session" && item.key === "b")).toMatchObject({ depth: 0 });
    expect(sessions.find(item => item.kind === "session" && item.key === "a")).toMatchObject({ depth: 1 });
  });
});
