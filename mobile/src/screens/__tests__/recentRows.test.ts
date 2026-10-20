import type { RemoteSession } from "../../remote/types";
import type { MergedSessionRow } from "../../remote/mergeSessions";
import { isActiveSession, recentRows } from "../recentRows";

const minute = 60_000;
const now = Date.UTC(2026, 9, 21, 12, 0, 0);

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

const ids = (rows: ReturnType<typeof recentRows>) =>
  rows.filter(item => item.kind === "session").map(item => (item.kind === "session" ? item.key : ""));
const captions = (rows: ReturnType<typeof recentRows>) =>
  rows.filter(item => item.kind === "section").map(item => (item.kind === "section" ? item.titleKey : ""));
const countOf = (rows: ReturnType<typeof recentRows>, key: string) =>
  rows.find(item => item.kind === "section" && item.key.endsWith(key)) as
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
    const rows = recentRows([
      row("idle-new", {}, now - minute),
      row("active-old", { streaming: true }, now - 10 * minute),
      row("idle-old", {}, now - 60 * minute),
      row("active-new", { status: "waiting_approval" }, now - 2 * minute),
    ], "");
    expect(captions(rows)).toEqual(["sessions.activeSection", "sessions.earlierSection"]);
    expect(ids(rows)).toEqual(["active-new", "active-old", "idle-new", "idle-old"]);
    // The captions carry what is under them, so a reader can see how much moves.
    expect(countOf(rows, "activeSection")!.count).toBe(2);
    expect(countOf(rows, "earlierSection")!.count).toBe(2);
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
    expect(recentRows([], "")).toEqual([]);
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

  it("draws a fork as a row of its own, at its own age", () => {
    const rows = recentRows([
      row("parent", {}, now - 30 * minute),
      row("fork", { parentSessionId: "parent" }, now - minute),
    ], "");
    expect(ids(rows)).toEqual(["fork", "parent"]);
  });
});
