import { describe, expect, it } from "vitest";
import { formatRequestedAction, parseAction, parseSaveSuggestion, unwrapNestedJson } from "./approvalPayload";

describe("parseAction", () => {
  it("returns null for null/undefined/empty payloads", () => {
    expect(parseAction(null)).toBeNull();
    expect(parseAction(undefined)).toBeNull();
    expect(parseAction("")).toBeNull();
  });

  it("returns null for non-JSON and non-object JSON", () => {
    expect(parseAction("not json {{{")).toBeNull();
    expect(parseAction("42")).toBeNull();
    expect(parseAction("\"a string\"")).toBeNull();
    expect(parseAction("[1,2,3]")).toBeNull();
    expect(parseAction("null")).toBeNull();
  });

  it("requires string tool and category", () => {
    expect(parseAction(JSON.stringify({ category: "shell_command" }))).toBeNull();
    expect(parseAction(JSON.stringify({ tool: "shell" }))).toBeNull();
    expect(parseAction(JSON.stringify({ category: 1, tool: "shell" }))).toBeNull();
  });

  it("parses a minimal valid action", () => {
    expect(parseAction(JSON.stringify({ category: "shell_command", tool: "shell" }))).toEqual({
      blockedPaths: undefined,
      category: "shell_command",
      command: undefined,
      deletes: undefined,
      justification: undefined,
      paths: undefined,
      scope: undefined,
      summary: undefined,
      tool: "shell",
      writes: undefined,
    });
  });

  it("drops optional fields with the wrong shape rather than passing them through", () => {
    const action = parseAction(JSON.stringify({
      blocked_paths: ["/a", 5],
      category: "file_write",
      command: 123,
      deletes: [{ path: 1 }],
      justification: "",
      paths: "not-an-array",
      scope: { cwd: "/w", estimatedBlastRadius: "nuclear", insideWorkspace: true },
      summary: { nested: true },
      tool: "write",
      writes: [{ path: "/ok", preview: 9 }],
    }));
    expect(action).toEqual({
      blockedPaths: undefined,
      category: "file_write",
      command: undefined,
      deletes: undefined,
      justification: undefined,
      paths: undefined,
      scope: undefined,
      summary: undefined,
      tool: "write",
      writes: undefined,
    });
  });

  it("keeps well-formed optional fields", () => {
    const action = parseAction(JSON.stringify({
      blocked_paths: ["/blocked"],
      category: "sandbox_escalation",
      command: "rm -rf /tmp/x",
      deletes: [{ path: "/gone" }],
      justification: "needs it",
      paths: ["/read/a"],
      scope: { cwd: "/w", estimatedBlastRadius: "high", insideWorkspace: false },
      summary: "does a thing",
      tool: "shell",
      writes: [{ path: "/w/a", preview: "hi" }, { path: "/w/b" }],
    }));
    expect(action).toEqual({
      blockedPaths: ["/blocked"],
      category: "sandbox_escalation",
      command: "rm -rf /tmp/x",
      deletes: [{ path: "/gone" }],
      justification: "needs it",
      paths: ["/read/a"],
      scope: { cwd: "/w", estimatedBlastRadius: "high", insideWorkspace: false },
      summary: "does a thing",
      tool: "shell",
      writes: [{ path: "/w/a", preview: "hi" }, { path: "/w/b" }],
    });
  });
});

describe("parseSaveSuggestion", () => {
  it("returns null for empty/malformed payloads", () => {
    expect(parseSaveSuggestion(null)).toBeNull();
    expect(parseSaveSuggestion("")).toBeNull();
    expect(parseSaveSuggestion("not json")).toBeNull();
    expect(parseSaveSuggestion("[]")).toBeNull();
    expect(parseSaveSuggestion(JSON.stringify({ access: "read" }))).toBeNull();
    expect(parseSaveSuggestion(JSON.stringify({ path: "/a" }))).toBeNull();
    expect(parseSaveSuggestion(JSON.stringify({ access: 1, path: "/a" }))).toBeNull();
  });

  it("parses a valid suggestion", () => {
    expect(parseSaveSuggestion(JSON.stringify({ access: "write", path: "/w/**" }))).toEqual({
      access: "write",
      path: "/w/**",
    });
  });
});

describe("unwrapNestedJson", () => {
  it("returns a non-string value unchanged", () => {
    const obj = { command: "ls" };
    expect(unwrapNestedJson(obj)).toBe(obj);
  });

  it("unwraps single, double, and triple JSON encodings", () => {
    expect(unwrapNestedJson(JSON.stringify({ command: "ls" }))).toEqual({ command: "ls" });
    expect(unwrapNestedJson(JSON.stringify(JSON.stringify({ command: "ls" })))).toEqual({ command: "ls" });
    expect(unwrapNestedJson(JSON.stringify(JSON.stringify(JSON.stringify({ command: "ls" }))))).toEqual({ command: "ls" });
  });

  it("stops after maxDepth even if the result is still a string", () => {
    // Four levels of encoding, unwrapped only three times → still a JSON string.
    const quad = JSON.stringify(JSON.stringify(JSON.stringify(JSON.stringify("x"))));
    expect(typeof unwrapNestedJson(quad)).toBe("string");
  });

  it("throws when an intermediate string is not valid JSON", () => {
    expect(() => unwrapNestedJson("ls -la")).toThrow();
  });
});

describe("formatRequestedAction", () => {
  it("returns an empty string for empty input", () => {
    expect(formatRequestedAction(null)).toBe("");
    expect(formatRequestedAction(undefined)).toBe("");
    expect(formatRequestedAction("")).toBe("");
  });

  it("returns the raw string when it is not JSON", () => {
    expect(formatRequestedAction("ls -la")).toBe("ls -la");
  });

  it("extracts .command from a (possibly nested) JSON object", () => {
    expect(formatRequestedAction(JSON.stringify({ command: "echo hi" }))).toBe("echo hi");
    expect(formatRequestedAction(JSON.stringify(JSON.stringify({ command: "echo hi" })))).toBe("echo hi");
  });

  it("pretty-prints a JSON object without a command", () => {
    expect(formatRequestedAction(JSON.stringify({ tool: "read" }))).toBe(
      JSON.stringify({ tool: "read" }, null, 2),
    );
  });
});
