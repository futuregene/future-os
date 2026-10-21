import { describe, expect, it } from "vitest";
import { moveInOrder, resolveOrder, vibeKeyOfLocal, withoutInOrder } from "./order";

const at = (key: string, createdAt: number) => ({ createdAt, key });
const local = (id: string, createdAt: number) => at(vibeKeyOfLocal(id), createdAt);

describe("vibeKeyOfLocal", () => {
  it("names a conversation by id, not by title", () => {
    // A rename must not move a cell.
    expect(vibeKeyOfLocal("t1")).toBe("local:t1");
  });
});

describe("resolveOrder", () => {
  it("defaults to creation time, oldest first", () => {
    const members = [local("c", 300), local("a", 100), local("b", 200)];
    expect(resolveOrder(members, [])).toEqual(["local:a", "local:b", "local:c"]);
  });

  it("keeps the user's relative order where they chose one", () => {
    const members = [local("a", 100), local("b", 200), local("c", 300)];
    expect(resolveOrder(members, ["local:c", "local:a", "local:b"]))
      .toEqual(["local:c", "local:a", "local:b"]);
  });

  it("puts a newly pinned conversation where it belongs in time", () => {
    // The user moved c and a to the front; b is pinned later but is *older* than
    // both, so it lands before them instead of at the end.
    const members = [local("a", 200), local("c", 300), local("b", 100)];
    expect(resolveOrder(members, ["local:c", "local:a"]))
      .toEqual(["local:b", "local:c", "local:a"]);
  });

  it("appends a newer pin after the ordered ones", () => {
    const members = [local("a", 100), local("b", 200), local("d", 400)];
    expect(resolveOrder(members, ["local:b", "local:a"])).toEqual(["local:b", "local:a", "local:d"]);
  });

  it("drops a key that is no longer a member without moving the others", () => {
    const members = [local("a", 100), local("b", 200)];
    expect(resolveOrder(members, ["local:b", "local:gone", "local:a"]))
      .toEqual(["local:b", "local:a"]);
  });

  it("ignores a duplicate and keeps the first position", () => {
    const members = [local("a", 100), local("b", 200)];
    expect(resolveOrder(members, ["local:b", "local:a", "local:b"]))
      .toEqual(["local:b", "local:a"]);
  });

  it("is stable for two conversations created in the same millisecond", () => {
    const same = [local("b", 100), local("a", 100)];
    expect(resolveOrder(same, [])).toEqual(["local:a", "local:b"]);
    // And the same answer whatever order the store handed them over in.
    expect(resolveOrder([...same].reverse(), [])).toEqual(["local:a", "local:b"]);
  });

  it("returns every member exactly once, whatever the input", () => {
    const members = [local("a", 100), local("b", 200), local("c", 300)];
    const order = resolveOrder(members, ["local:zzz", "local:c", "local:c", "", "local:a"]);
    expect(order).toHaveLength(3);
    expect(new Set(order).size).toBe(3);
  });
});

describe("moveInOrder", () => {
  const order = ["local:a", "local:b", "local:c"];

  it("moves one place in either direction", () => {
    expect(moveInOrder(order, "local:b", -1)).toEqual(["local:b", "local:a", "local:c"]);
    expect(moveInOrder(order, "local:b", 1)).toEqual(["local:a", "local:c", "local:b"]);
  });

  it("stays put at either end", () => {
    expect(moveInOrder(order, "local:a", -1)).toEqual(order);
    expect(moveInOrder(order, "local:c", 1)).toEqual(order);
  });

  it("does nothing for a key that is not in the order", () => {
    expect(moveInOrder(order, "local:zzz", 1)).toEqual(order);
  });

  it("never mutates its input", () => {
    const input = [...order];
    moveInOrder(input, "local:a", 1);
    expect(input).toEqual(order);
  });
});

describe("withoutInOrder", () => {
  it("removes only that key", () => {
    expect(withoutInOrder(["local:a", "local:b"], "local:a")).toEqual(["local:b"]);
    expect(withoutInOrder(["local:a", "local:b"], "local:zzz")).toEqual(["local:a", "local:b"]);
  });
});
