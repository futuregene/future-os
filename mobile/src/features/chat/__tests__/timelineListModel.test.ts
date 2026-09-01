import { newestFirst } from "../timelineListModel";

describe("timeline list data model", () => {
  test("an older chronological prepend only appends in inverted view space", () => {
    const current = ["recent-1", "recent-2"];
    const before = newestFirst(current);
    const after = newestFirst(["older-1", "older-2", ...current]);

    expect(before).toEqual(["recent-2", "recent-1"]);
    expect(after.slice(0, before.length)).toEqual(before);
    expect(after.slice(before.length)).toEqual(["older-2", "older-1"]);
    expect(current).toEqual(["recent-1", "recent-2"]);
  });
});
