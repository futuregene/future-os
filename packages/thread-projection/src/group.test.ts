import { describe, expect, it } from "vitest";
import {
  asToolKind,
  dedupeByTarget,
  foldCollapsibleRuns,
  isToolKind,
  normalizeArgs,
  targetFromArgs,
} from "./group";
import type { ToolKind } from "./group";

describe("tool kinds", () => {
  it.each<[string, boolean]>([
    ["read", true],
    ["shell", true],
    ["edit", true],
    ["write", true],
    // "thinking" is not a tool, and unknown tools are not modeled kinds.
    ["thinking", false],
    ["Read", false],
    ["", false],
    ["grep", false],
  ])("isToolKind(%j) === %s", (name, expected) => {
    expect(isToolKind(name)).toBe(expected);
  });

  it("defaults unknown tool names to shell", () => {
    expect(asToolKind("read")).toBe("read");
    expect(asToolKind("edit")).toBe("edit");
    expect(asToolKind("write")).toBe("write");
    expect(asToolKind("bash")).toBe("shell");
    expect(asToolKind("")).toBe("shell");
  });
});

describe("normalizeArgs", () => {
  it("passes a record through and parses JSON strings", () => {
    const record = { path: "a.md" };
    expect(normalizeArgs(record)).toBe(record);
    expect(normalizeArgs('{"path":"a.md"}')).toEqual({ path: "a.md" });
  });

  it("unwraps nested JSON strings up to the documented depth", () => {
    const once = JSON.stringify({ path: "a.md" });
    expect(normalizeArgs(JSON.stringify(once))).toEqual({ path: "a.md" });
    const twice = JSON.stringify(once);
    expect(normalizeArgs(JSON.stringify(twice))).toEqual({ path: "a.md" });
    // A fourth encoding is past the limit and is left as an unparsed string.
    const thrice = JSON.stringify(twice);
    expect(normalizeArgs(JSON.stringify(thrice))).toBeNull();
  });

  it.each<[unknown, null]>([
    [null, null],
    [undefined, null],
    [42, null],
    [true, null],
    [[1], null],
    ["", null],
    ["   ", null],
    ["not json", null],
    ["42", null],
  ])("rejects %j", (value, expected) => {
    expect(normalizeArgs(value)).toBe(expected);
  });
});

describe("targetFromArgs", () => {
  it("reads the command for shell and the path for file tools", () => {
    expect(targetFromArgs("shell", { command: "ls -la" })).toBe("ls -la");
    expect(targetFromArgs("read", { path: "a.md" })).toBe("a.md");
    expect(targetFromArgs("edit", { file_path: "b.md" })).toBe("b.md");
    expect(targetFromArgs("write", { filePath: "c.md" })).toBe("c.md");
    expect(targetFromArgs("read", { command: "ls" })).toBeUndefined();
    expect(targetFromArgs("shell", { path: "a.md" })).toBeUndefined();
    expect(targetFromArgs("shell", { command: 42 })).toBeUndefined();
    expect(targetFromArgs("read", null)).toBeUndefined();
  });
});

describe("dedupeByTarget", () => {
  it("keeps the first item per target and falls back to the id", () => {
    expect(
      dedupeByTarget([
        { id: "1", target: "a.md" },
        { id: "2", target: "a.md" },
        { id: "3", target: "b.md" },
        { id: "4" },
        { id: "4" },
        { id: "5" },
      ]),
    ).toEqual([{ id: "1", target: "a.md" }, { id: "3", target: "b.md" }, { id: "4" }, { id: "5" }]);
  });
});

describe("foldCollapsibleRuns", () => {
  type Item = { id: string; kind: ToolKind | null };
  const kindOf = (item: Item) => item.kind;
  const item = (id: string, kind: ToolKind | null): Item => ({ id, kind });

  it("folds only runs of more than one consecutive same-kind item", () => {
    const runs = foldCollapsibleRuns(
      [item("a", "edit"), item("b", "edit"), item("c", "read"), item("d", null), item("e", "edit"), item("f", "shell"), item("g", "shell")],
      kindOf,
    );
    expect(runs).toEqual([
      { collapsed: true, group: [item("a", "edit"), item("b", "edit")], kind: "edit" },
      { collapsed: false, item: item("c", "read") },
      { collapsed: false, item: item("d", null) },
      { collapsed: false, item: item("e", "edit") },
      { collapsed: true, group: [item("f", "shell"), item("g", "shell")], kind: "shell" },
    ]);
  });

  it("returns nothing for an empty input and a single item unchanged", () => {
    expect(foldCollapsibleRuns<Item>([], kindOf)).toEqual([]);
    expect(foldCollapsibleRuns([item("a", "write")], kindOf)).toEqual([
      { collapsed: false, item: item("a", "write") },
    ]);
  });

  it("breaks a run when the kind changes even if both are collapsible", () => {
    expect(foldCollapsibleRuns([item("a", "read"), item("b", "read"), item("c", "shell")], kindOf)).toEqual([
      { collapsed: true, group: [item("a", "read"), item("b", "read")], kind: "read" },
      { collapsed: false, item: item("c", "shell") },
    ]);
  });
});
