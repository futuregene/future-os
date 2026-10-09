import type { MergedSessionRow, CatalogEntry, DesktopFilter } from "../mergeSessions";
import { describe, expect, it } from "@jest/globals";
import { hiddenSessionCount, mergeSessions, sessionKey } from "../mergeSessions";
import type { RemoteSession } from "../types";

function session(overrides: Partial<RemoteSession> & { sessionId: string }): RemoteSession {
  return { title: overrides.sessionId, streaming: false, threadId: overrides.sessionId, ...overrides };
}

function catalog(desktopId: string, sessions: RemoteSession[]): CatalogEntry {
  return { desktopId, label: desktopId, sessions };
}

const ALL: DesktopFilter = { kind: "all" };

describe("mergeSessions", () => {
  it("interleaves desktops by time rather than grouping them", () => {
    const merged = mergeSessions([
      catalog("a", [session({ sessionId: "a_old", lastMessageAt: 100 })]),
      catalog("b", [session({ sessionId: "b_mid", lastMessageAt: 500 })]),
      catalog("c", [session({ sessionId: "c_new", lastMessageAt: 900 })]),
    ], ALL);

    expect(merged.map(row => row.session.sessionId)).toEqual(["c_new", "b_mid", "a_old"]);
  });

  /**
   * Session ids are unique per desktop, not globally. Two machines holding the
   * same id must stay two rows, each labelled with its own desktop — a row that
   * lost its source would be opened against the wrong machine.
   */
  it("keeps same-id sessions from different desktops apart", () => {
    const merged = mergeSessions([
      catalog("a", [session({ sessionId: "shared", lastMessageAt: 300 })]),
      catalog("b", [session({ sessionId: "shared", lastMessageAt: 200 })]),
    ], ALL);

    expect(merged).toHaveLength(2);
    expect(merged.map(row => row.desktopId)).toEqual(["a", "b"]);
    expect(new Set(merged.map(row => row.key)).size).toBe(2);
    expect(merged[0]!.key).toBe(sessionKey("a", "shared"));
  });

  it("ranks pinned sessions above newer unpinned ones", () => {
    const merged = mergeSessions([
      catalog("a", [
        session({ sessionId: "old_pin", pinned: true, lastMessageAt: 10 }),
        session({ sessionId: "fresh", lastMessageAt: 10_000 }),
      ]),
    ], ALL);
    expect(merged.map(row => row.session.sessionId)).toEqual(["old_pin", "fresh"]);
  });

  /**
   * A desktop older than the `lastMessageAt` field sends sessions without one.
   * Treating unknown age as "now" would float a stale conversation to the top.
   */
  it("sorts sessions without a timestamp last", () => {
    const merged = mergeSessions([
      catalog("a", [session({ sessionId: "undated" })]),
      catalog("b", [session({ sessionId: "dated", lastMessageAt: 5 })]),
    ], ALL);
    expect(merged.map(row => row.session.sessionId)).toEqual(["dated", "undated"]);
    expect(merged[1]!.lastMessageAt).toBeNull();
  });

  it("is stable for equal timestamps so a poll cannot reshuffle the list", () => {
    const merged = mergeSessions([
      catalog("b", [session({ sessionId: "s", lastMessageAt: 7 })]),
      catalog("a", [session({ sessionId: "s", lastMessageAt: 7 })]),
    ], ALL);
    expect(merged.map(row => row.desktopId)).toEqual(["a", "b"]);
  });

  it("filters to one desktop without touching the others", () => {
    const catalogs = [
      catalog("a", [session({ sessionId: "a1" })]),
      catalog("b", [session({ sessionId: "b1" })]),
    ];
    expect(mergeSessions(catalogs, { kind: "desktop", desktopId: "a" }))
      .toHaveLength(1);
    expect(mergeSessions(catalogs, { kind: "desktop", desktopId: "a" })[0]!.desktopId).toBe("a");
    expect(hiddenSessionCount(catalogs, { kind: "all" })).toBe(0);
    expect(hiddenSessionCount(catalogs, { kind: "desktop", desktopId: "a" })).toBe(1);
  });

  /** A desktop that has not answered yet contributes nothing, not stale rows. */
  it("omits a desktop with no catalogue", () => {
    expect(mergeSessions([catalog("a", [])], ALL)).toEqual([]);
  });

  it("de-duplicates a session that appears twice in one catalogue", () => {
    const merged = mergeSessions([
      catalog("a", [
        session({ sessionId: "dup", title: "first", lastMessageAt: 1 }),
        session({ sessionId: "dup", title: "second", lastMessageAt: 2 }),
      ]),
    ], ALL);
    expect(merged).toHaveLength(1);
    expect(merged[0]!.session.title).toBe("first");
  });
});
