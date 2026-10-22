import type { RemoteSession } from "../../remote/types";
import { listTargets, selectableTargets } from "../sessionSelection";
import type { MergedSessionRow } from "../../remote/mergeSessions";

const session = (id: string, extra: Partial<RemoteSession> = {}): RemoteSession => ({
  sessionId: id,
  threadId: `t-${id}`,
  title: id,
  streaming: false,
  ...extra,
});
const merged = (desktopId: string, id: string, extra: Partial<RemoteSession> = {}): MergedSessionRow => ({
  key: `${desktopId}::${id}`,
  desktopId,
  session: session(id, extra),
  lastMessageAt: null,
});
const answers = () => ({ capabilities: new Set<string>() }) as never;

describe("the rows a batch addresses", () => {
  it("keys a single machine's conversations by their own id", () => {
    expect(listTargets(undefined, [session("a"), session("b")])).toEqual([
      { key: "a", session: session("a"), desktopId: null },
      { key: "b", session: session("b"), desktopId: null },
    ]);
  });

  it("keys a merged list by machine and id, so two machines' same id stay apart", () => {
    const targets = listTargets([merged("desk-a", "s1"), merged("desk-b", "s1")], []);
    expect(targets.map(target => target.key)).toEqual(["desk-a::s1", "desk-b::s1"]);
    expect(targets.map(target => target.desktopId)).toEqual(["desk-a", "desk-b"]);
  });

  it("reads an empty merged list as empty rather than falling back to one machine", () => {
    // The screen that names a machine with no rows of its own says so; falling
    // back here would hand a batch conversations of a machine it is not showing.
    expect(listTargets([], [session("a")])).toEqual([]);
  });
});

describe("what a batch may hold", () => {
  const targets = listTargets(undefined, [
    session("plain"),
    session("pinned", { pinned: true }),
  ]);

  it("skips pinned shortcuts on the tree pages, where a pin lifts the row out of its group", () => {
    expect(selectableTargets(targets, answers, true).map(target => target.session.sessionId))
      .toEqual(["plain"]);
  });

  it("keeps a pinned conversation on the recency page, where it is an ordinary row", () => {
    // That page ignores pinning for its order and only draws the marker, so
    // there is no shortcut section to have lifted the row out of.
    expect(selectableTargets(targets, answers, false).map(target => target.session.sessionId))
      .toEqual(["plain", "pinned"]);
  });

  it("skips a machine that cannot take the delete, whatever the page", () => {
    expect(selectableTargets(targets, () => null, true)).toEqual([]);
    expect(selectableTargets(targets, () => null, false)).toEqual([]);
  });
});
