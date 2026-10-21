import type { StoredThread } from "../../integrations/storage/threadStore";
import type { RemoteCatalog } from "../remote-peer/mergeConversations";
import { describe, expect, it } from "vitest";
import { vibeCellsOf, vibeKeyOfRemote, vibeMachines } from "./vibeCells";

/**
 * Membership across machines: which conversations the panel shows, and which machine
 * filter lets them through.
 *
 * Membership is still the pin — a host's catalogue reports its own `pinned`, and the
 * panel reads it rather than keeping a second list. What changes with a second
 * machine is that the *filter* is now part of the question, so it is answered here
 * (pure) instead of inside the component.
 */
function thread(id: string, over: Partial<StoredThread> = {}): StoredThread {
  return {
    createdAt: 100,
    id,
    mode: "chat",
    pinned: true,
    readonly: false,
    status: "active",
    title: `Local ${id}`,
    updatedAt: 100,
    workspaceId: "ws1",
    ...over,
  };
}

const catalog = (desktopId: string, sessions: RemoteCatalog["sessions"]): RemoteCatalog => ({ desktopId, sessions });

const workspaceNames = new Map([["ws1", "Writing"]]);

describe("vibeCellsOf", () => {
  it("takes pinned local conversations with their creation time", () => {
    const cells = vibeCellsOf([thread("a", { createdAt: 42 })], [], { kind: "all" }, workspaceNames);
    expect(cells).toHaveLength(1);
    expect(cells[0]).toMatchObject({
      createdAt: 42,
      desktopId: null,
      key: "local:a",
      kind: "local",
      title: "Local a",
      workspaceName: "Writing",
    });
  });

  it("takes a host's pinned conversations, and says when it cannot date them", () => {
    // A host's catalogue carries no creation time, so the cell says `null` rather than
    // inventing one: `order.ts` puts an undated member where the user can move it.
    const cells = vibeCellsOf([], [catalog("desk_1", [
      { pinned: true, sessionId: "s1", status: "active", title: "Remote one", workspaceName: "Lab" },
    ])], { kind: "all" }, workspaceNames);
    expect(cells).toEqual([{
      createdAt: null,
      desktopId: "desk_1",
      key: "remote:desk_1:s1",
      kind: "remote",
      session: { pinned: true, sessionId: "s1", status: "active", title: "Remote one", workspaceName: "Lab" },
      title: "Remote one",
      workspaceName: "Lab",
    }]);
  });

  it("leaves out an unpinned or archived conversation on either machine", () => {
    const cells = vibeCellsOf(
      [thread("live"), thread("unpinned", { pinned: false }), thread("archived", { status: "archived" })],
      [catalog("desk_1", [
        { pinned: false, sessionId: "s1", title: "unpinned remote" },
        { pinned: true, sessionId: "s2", status: "archived", title: "archived remote" },
        { pinned: true, sessionId: "s3", status: "active", title: "live remote" },
      ])],
      { kind: "all" },
      workspaceNames,
    );
    expect(cells.map(cell => cell.key)).toEqual(["local:live", "remote:desk_1:s3"]);
  });

  it("keeps a host that reports no status", () => {
    // An older host says nothing about status; silence is not "archived".
    const cells = vibeCellsOf([], [catalog("desk_1", [{ pinned: true, sessionId: "s1", title: "old host" }])], { kind: "all" }, workspaceNames);
    expect(cells).toHaveLength(1);
  });

  it("shows one machine when the filter names it", () => {
    const threads = [thread("a")];
    const catalogs = [catalog("desk_1", [{ pinned: true, sessionId: "s1", title: "one" }])];
    expect(vibeCellsOf(threads, catalogs, { desktopId: "desk_1", kind: "device" }, workspaceNames).map(cell => cell.key))
      .toEqual(["remote:desk_1:s1"]);
    expect(vibeCellsOf(threads, catalogs, { desktopId: "local", kind: "device" }, workspaceNames).map(cell => cell.key))
      .toEqual(["local:a"]);
  });

  it("has nothing for a machine with no pins", () => {
    expect(vibeCellsOf([], [catalog("desk_1", [])], { desktopId: "desk_1", kind: "device" }, workspaceNames)).toEqual([]);
  });
});

describe("vibeMachines", () => {
  it("offers this machine first, then the paired hosts by name", () => {
    // Built from what is *paired*, not from what has pins: a filter that only listed
    // machines with pinned conversations would vanish exactly when the panel is empty.
    const machines = vibeMachines(
      [catalog("desk_1", []), catalog("desk_2", [])],
      new Map([["desk_1", "Studio"]]),
    );
    expect(machines).toEqual([
      { id: "local", label: "" },
      { id: "desk_1", label: "Studio" },
      // A host with no label yet falls back to its id, which is still an answer.
      { id: "desk_2", label: "desk_2" },
    ]);
  });
});

describe("vibeKeyOfRemote", () => {
  it("names a remote conversation by ids, so a rename cannot move it", () => {
    expect(vibeKeyOfRemote("desk_1", "s1")).toBe("remote:desk_1:s1");
  });
});
