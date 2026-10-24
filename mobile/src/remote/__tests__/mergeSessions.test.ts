import type { CatalogEntry, DesktopFilter } from "../mergeSessions";
import { describe, expect, it } from "@jest/globals";
import { hiddenSessionCount, mergeSessions, sessionKey } from "../mergeSessions";
import type { MachineStatus } from "../desktopStatus";
import type { RemoteSession } from "../types";

function session(overrides: Partial<RemoteSession> & { sessionId: string }): RemoteSession {
  return { title: overrides.sessionId, streaming: false, threadId: overrides.sessionId, ...overrides };
}

function catalog(
  desktopId: string,
  sessions: RemoteSession[],
  status: MachineStatus = "connected",
): CatalogEntry {
  return { desktopId, label: desktopId, status, sessions };
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

  /**
   * A machine that is not answering contributes nothing either — its cached rows
   * outlive the connection that delivered them.
   *
   * The catalogue a phone holds for a desktop is the last one it sent, and it is
   * still here after the socket goes: a row drawn from it names a conversation
   * this phone cannot open until that machine answers again. Only `connected`
   * draws, so `connecting` (a handshake in flight), `preparing` (the desktop's
   * agent restarting) and `offline` all leave the list to the machines that can
   * actually be read.
   */
  it("draws only machines that are connected", () => {
    const catalogs = [
      catalog("up", [session({ sessionId: "up1", lastMessageAt: 100 })], "connected"),
      catalog("dialing", [session({ sessionId: "dial1", lastMessageAt: 300 })], "connecting"),
      catalog("warming", [session({ sessionId: "warm1", lastMessageAt: 200 })], "preparing"),
      catalog("gone", [session({ sessionId: "gone1", lastMessageAt: 400 })], "offline"),
    ];

    expect(mergeSessions(catalogs, ALL).map(row => row.session.sessionId)).toEqual(["up1"]);
    // Naming a machine that is not answering shows none of its rows either: the
    // screen's own answer for that machine is its status, not its old list.
    expect(mergeSessions(catalogs, { kind: "desktop", desktopId: "gone" })).toEqual([]);
    // And the hidden count speaks about the filter, never about a machine nobody
    // can read: the rows of a down machine are in neither list.
    expect(hiddenSessionCount(catalogs, { kind: "all" })).toBe(0);
    expect(hiddenSessionCount(catalogs, { kind: "desktop", desktopId: "up" })).toBe(0);
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
