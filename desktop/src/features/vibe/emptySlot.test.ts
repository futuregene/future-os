import type { StoredThread } from "../../integrations/storage/threadStore";
import type { RemoteCatalog } from "../remote-peer/mergeConversations";
import { describe, expect, it } from "vitest";
import { clampPage, hasAddSlot, pageCount, pageSlice, recentChoices } from "./emptySlot";

/**
 * The two "what happens when the grid has room / has no room" rules.
 *
 * The page number is the answer to a question the panel used to answer with a silent cap:
 * a thirteenth pinned conversation used to be unreachable. The recent list is the answer to
 * "what can I even add", which a picker that only lists everything is bad at.
 */
function thread(id: string, over: Partial<StoredThread> = {}): StoredThread {
  return {
    createdAt: 1,
    id,
    mode: "chat",
    pinned: false,
    readonly: false,
    status: "active",
    title: `T ${id}`,
    updatedAt: 1,
    workspaceId: "ws",
    ...over,
  };
}

describe("pageCount / clampPage", () => {
  it("counts pages twelve at a time, and always at least one", () => {
    expect(pageCount(0)).toBe(1);
    expect(pageCount(1)).toBe(1);
    expect(pageCount(12)).toBe(1);
    expect(pageCount(13)).toBe(2);
    expect(pageCount(25)).toBe(3);
  });

  it("keeps the page inside the range, so an unpin cannot strand the reader", () => {
    // Pinning 13, opening page two, unpinning back to 12: the page must fall back to one
    // rather than showing an empty grid.
    expect(clampPage(1, 13)).toBe(1);
    expect(clampPage(1, 12)).toBe(0);
    expect(clampPage(5, 3)).toBe(0);
    expect(clampPage(-2, 30)).toBe(0);
  });
});

describe("pageSlice", () => {
  it("slices by page and clamps an out-of-range page", () => {
    const cells = Array.from({ length: 14 }, (_, index) => index);
    expect(pageSlice(cells, 0)).toHaveLength(12);
    expect(pageSlice(cells, 1)).toEqual([12, 13]);
    expect(pageSlice(cells, 9)).toEqual([12, 13]);
  });
});

describe("hasAddSlot", () => {
  it("offers the tile while the page has room, and not when it is full", () => {
    expect(hasAddSlot(0)).toBe(true);
    expect(hasAddSlot(11)).toBe(true);
    // A full page has nowhere to put the tile; the menu's add row is the way in there.
    expect(hasAddSlot(12)).toBe(false);
  });
});

describe("recentChoices", () => {
  const catalog = (desktopId: string, sessions: Array<Record<string, unknown>>) =>
    ({ desktopId, sessions } as unknown as RemoteCatalog);

  it("lists unpinned conversations by when they were last touched", () => {
    // The panel's own order is creation time; this list is deliberately the opposite,
    // because "I was just in this one" is the question a picker answers.
    const choices = recentChoices(
      [
        thread("old", { lastMessageAt: 100 }),
        thread("recent", { lastMessageAt: 900 }),
        thread("pinned", { pinned: true, lastMessageAt: 999 }),
        thread("archived", { status: "archived", lastMessageAt: 950 }),
      ],
      [],
      new Set(),
      new Map(),
    );
    expect(choices.map(choice => choice.key)).toEqual(["local:recent", "local:old"]);
  });

  it("falls back to when a conversation was opened, then to when it was edited", () => {
    const choices = recentChoices(
      [
        thread("opened", { lastOpenedAt: 500, lastMessageAt: null, updatedAt: 10 }),
        thread("edited", { lastOpenedAt: null, lastMessageAt: null, updatedAt: 800 }),
      ],
      [],
      new Set(),
      new Map(),
    );
    expect(choices.map(choice => choice.key)).toEqual(["local:edited", "local:opened"]);
  });

  it("leaves out what the panel already shows, and names the machine for a host's", () => {
    // The host's rows carry its last run's status, not a lifecycle (`vibeCells.ts`), so
    // a conversation that has already run is still a candidate to add.
    const choices = recentChoices(
      [],
      [catalog("desk_1", [
        { lastMessageAt: 700, pinned: false, sessionId: "s1", status: "completed", title: "Remote one" },
        { lastMessageAt: 800, pinned: true, sessionId: "s2", status: "completed", title: "Already pinned" },
        { lastMessageAt: 900, pinned: false, sessionId: "s3", status: "completed", title: "Gone" },
      ])],
      new Set(["remote:desk_1:s3"]),
      new Map([["desk_1", "Studio"]]),
    );
    expect(choices).toEqual([{ key: "remote:desk_1:s1", machine: "Studio", title: "Remote one" }]);
  });

  it("keeps the list short", () => {
    const many = Array.from({ length: 20 }, (_, index) => thread(`t${index}`, { lastMessageAt: index }));
    expect(recentChoices(many, [], new Set(), new Map())).toHaveLength(5);
  });
});
