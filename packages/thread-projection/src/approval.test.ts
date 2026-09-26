import { describe, expect, it } from "vitest";
import {
  approvalCommand,
  approvalDeletes,
  approvalPaths,
  escalationTitleKey,
  formatRequestedAction,
  parseAction,
  parseSaveSuggestion,
  unwrapNestedJson,
} from "./approval";

describe("parseAction", () => {
  it.each<[unknown]>([[null], [undefined], [""], [0], [false], ["not json"], ["42"], ["[]"], ["{}"], [{ tool: "shell" }], [{ category: "shell" }]])(
    "rejects %j",
    (payload) => {
      expect(parseAction(payload)).toBeNull();
    },
  );

  it("parses a JSON string payload and keeps every valid field", () => {
    const action = parseAction(
      JSON.stringify({
        behavior: "manage_files",
        blocked_paths: ["/etc"],
        category: "sandbox_escalation",
        command: "rm -rf build",
        deletes: [{ path: "build" }],
        escalation_trigger: "sandbox_failure",
        justification: "needs the build dir",
        paths: ["a.md", "b.md"],
        scope: { cwd: "/repo", estimatedBlastRadius: "high", insideWorkspace: false },
        summary: "delete build output",
        targets: [{ path: "build", scope: "subtree" }],
        tool: "shell",
        writes: [{ path: "a.md", preview: "x" }, { path: "b.md" }],
      }),
    );
    expect(action).toEqual({
      behavior: "manage_files",
      blockedPaths: ["/etc"],
      category: "sandbox_escalation",
      command: "rm -rf build",
      deletes: [{ path: "build" }],
      escalationTrigger: "sandbox_failure",
      justification: "needs the build dir",
      paths: ["a.md", "b.md"],
      scope: { cwd: "/repo", estimatedBlastRadius: "high", insideWorkspace: false },
      summary: "delete build output",
      targets: [{ path: "build", scope: "subtree" }],
      tool: "shell",
      writes: [{ path: "a.md", preview: "x" }, { path: "b.md" }],
    });
  });

  it("accepts an already-parsed object", () => {
    expect(parseAction({ category: "edit", tool: "edit" })).toMatchObject({ category: "edit", tool: "edit" });
  });

  it("drops each malformed optional field instead of trusting it", () => {
    const action = parseAction({
      blocked_paths: ["ok", 2],
      behavior: "something_else",
      category: "edit",
      command: 42,
      deletes: [{ path: 1 }],
      escalation_trigger: "model_request",
      justification: "",
      paths: "nope",
      scope: { cwd: "/repo", estimatedBlastRadius: "extreme", insideWorkspace: true },
      summary: 7,
      targets: "nope",
      tool: "edit",
      writes: [{ preview: "no path" }],
    });
    expect(action).toEqual({
      behavior: undefined,
      blockedPaths: undefined,
      category: "edit",
      command: undefined,
      deletes: undefined,
      escalationTrigger: undefined,
      justification: undefined,
      paths: undefined,
      scope: undefined,
      summary: undefined,
      targets: undefined,
      tool: "edit",
      writes: undefined,
    });
  });

  it("validates the windows write capability payload strictly", () => {
    expect(parseAction({ category: "windows_write_capability", tool: "edit" })).toBeNull();
    expect(parseAction({ behavior: "manage_files", category: "windows_write_capability", tool: "edit" })).toBeNull();
    expect(
      parseAction({ behavior: "manage_files", category: "windows_write_capability", targets: [{ path: "a", scope: "file" }], tool: "edit" }),
    ).toMatchObject({ targets: [{ path: "a", scope: "file" }] });

    // Empty, over-long and malformed target lists are all rejected.
    for (const targets of [
      [],
      Array.from({ length: 9 }, (_, index) => ({ path: `p${index}`, scope: "file" as const })),
      [{ path: "", scope: "file" }],
      [{ path: "a", scope: "directory" }],
      ["a"],
    ]) {
      expect(parseAction({ behavior: "manage_files", category: "windows_write_capability", targets, tool: "edit" })).toBeNull();
    }
  });

  it("keeps the escalation trigger only for a sandbox escalation", () => {
    expect(parseAction({ category: "edit", escalation_trigger: "model_request", tool: "edit" })?.escalationTrigger).toBeUndefined();
    expect(parseAction({ category: "sandbox_escalation", escalation_trigger: "model_request", tool: "shell" })?.escalationTrigger).toBe("model_request");
    expect(parseAction({ category: "sandbox_escalation", escalation_trigger: "other", tool: "shell" })?.escalationTrigger).toBeUndefined();
  });

  it("accepts a scope only when every component is valid", () => {
    const valid = { cwd: "/repo", estimatedBlastRadius: "low", insideWorkspace: true };
    expect(parseAction({ category: "edit", scope: valid, tool: "edit" })?.scope).toEqual(valid);
    expect(parseAction({ category: "edit", scope: { ...valid, cwd: 1 }, tool: "edit" })?.scope).toBeUndefined();
    expect(parseAction({ category: "edit", scope: { ...valid, insideWorkspace: "yes" }, tool: "edit" })?.scope).toBeUndefined();
  });
});

describe("escalationTitleKey", () => {
  it("names the trigger-specific title and falls back to the neutral one", () => {
    expect(escalationTitleKey(null)).toBe("approval.escalationTitle");
    expect(escalationTitleKey({ category: "edit", tool: "edit" })).toBe("approval.escalationTitle");
    expect(escalationTitleKey({ category: "sandbox_escalation", escalationTrigger: "model_request", tool: "shell" })).toBe("approval.escalationRequestTitle");
    expect(escalationTitleKey({ category: "sandbox_escalation", escalationTrigger: "sandbox_failure", tool: "shell" })).toBe("approval.escalationRetryTitle");
    expect(escalationTitleKey({ category: "sandbox_escalation", tool: "shell" })).toBe("approval.escalationTitle");
  });
});

describe("parseSaveSuggestion", () => {
  it.each<[string | null | undefined]>([[null], [undefined], [""], ["not json"], ["42"], ["{}"], ['{"path":"a.md"}'], ['{"path":"a.md","access":42}']])(
    "rejects %j",
    (payload) => {
      expect(parseSaveSuggestion(payload)).toBeNull();
    },
  );

  it("parses the single-rule and rule-list shapes", () => {
    expect(parseSaveSuggestion('{"path":"a.md","access":"read"}')).toEqual({ access: "read", path: "a.md" });
    expect(parseSaveSuggestion('{"rules":[{"path":"a.md","access":"write"}]}')).toEqual({
      rules: [{ access: "write", path: "a.md" }],
    });
  });

  it("rejects rule lists that are empty, oversized or malformed", () => {
    expect(parseSaveSuggestion('{"rules":[]}')).toBeNull();
    expect(parseSaveSuggestion(JSON.stringify({ rules: Array.from({ length: 9 }, () => ({ access: "read", path: "p" })) }))).toBeNull();
    expect(parseSaveSuggestion('{"rules":[{"path":"","access":"read"}]}')).toBeNull();
    expect(parseSaveSuggestion('{"rules":[{"path":"a","access":"execute"}]}')).toBeNull();
  });
});

describe("unwrapNestedJson", () => {
  it("returns a non-string value immediately", () => {
    const record = { a: 1 };
    expect(unwrapNestedJson(record)).toBe(record);
    expect(unwrapNestedJson(42)).toBe(42);
    expect(unwrapNestedJson(null)).toBeNull();
  });

  it("unwraps up to maxDepth encodings and stops at the limit", () => {
    expect(unwrapNestedJson('{"a":1}')).toEqual({ a: 1 });
    expect(unwrapNestedJson(JSON.stringify(JSON.stringify({ a: 1 })))).toEqual({ a: 1 });
    // With maxDepth 1 a doubly-encoded value is only unwrapped once.
    expect(unwrapNestedJson(JSON.stringify('{"a":1}'), 1)).toBe('{"a":1}');
    // Four encodings exceed the default of three and stay a string.
    const deep = JSON.stringify(JSON.stringify(JSON.stringify(JSON.stringify({ a: 1 }))));
    expect(typeof unwrapNestedJson(deep)).toBe("string");
  });

  it("throws on a string that is not JSON (callers decide)", () => {
    expect(() => unwrapNestedJson("not json")).toThrow();
  });
});

describe("formatRequestedAction", () => {
  it("returns an empty string for a missing action", () => {
    expect(formatRequestedAction(null)).toBe("");
    expect(formatRequestedAction(undefined)).toBe("");
    expect(formatRequestedAction("")).toBe("");
  });

  it("prefers a command and otherwise pretty-prints the payload", () => {
    expect(formatRequestedAction('{"command":"ls -la","tool":"shell"}')).toBe("ls -la");
    expect(formatRequestedAction('{"tool":"shell"}')).toBe('{\n  "tool": "shell"\n}');
  });

  it("unwraps nested encodings and falls back to the raw text when unparsable", () => {
    expect(formatRequestedAction(JSON.stringify('{"command":"pwd"}'))).toBe("pwd");
    expect(formatRequestedAction("not json")).toBe("not json");
    expect(formatRequestedAction('{"command":42}')).toBe('{\n  "command": 42\n}');
  });
});

describe("approval payload accessors", () => {
  it("prefers writes, then targets, then paths", () => {
    expect(approvalPaths({ action: { paths: ["p"], targets: [{ path: "t" }], writes: [{ path: "w" }] } })).toEqual(["w"]);
    expect(approvalPaths({ action: { paths: ["p"], targets: [{ path: "t" }] } })).toEqual(["t"]);
    expect(approvalPaths({ action: { paths: ["p"] } })).toEqual(["p"]);
  });

  it("tolerates a JSON string action and drops unusable entries", () => {
    expect(approvalPaths({ action: '{"writes":[{"path":"a.md"}]}' })).toEqual(["a.md"]);
    expect(approvalPaths({ action: '{"paths":["a.md","",42]}' })).toEqual(["a.md"]);
    expect(approvalPaths({ action: '{"writes":"nope"}' })).toEqual([]);
    expect(approvalPaths({})).toEqual([]);
    expect(approvalPaths({ action: "not json" })).toEqual([]);
    expect(approvalPaths({ action: 42 })).toEqual([]);
    expect(approvalPaths({ action: null })).toEqual([]);
  });

  it("reads delete paths independently of writes", () => {
    expect(approvalDeletes({ action: { deletes: [{ path: "build" }, { path: "" }], writes: [{ path: "a.md" }] } })).toEqual(["build"]);
    expect(approvalDeletes({ action: "not json" })).toEqual([]);
    expect(approvalDeletes({})).toEqual([]);
  });

  it("reads the command, requiring a non-empty string", () => {
    expect(approvalCommand({ action: { command: "ls" } })).toBe("ls");
    expect(approvalCommand({ action: '{"command":"pwd"}' })).toBe("pwd");
    expect(approvalCommand({ action: { command: "" } })).toBeNull();
    expect(approvalCommand({ action: { command: 42 } })).toBeNull();
    expect(approvalCommand({ action: "not json" })).toBeNull();
    expect(approvalCommand({})).toBeNull();
  });
});
