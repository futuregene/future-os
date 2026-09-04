import {
  escalationTitleKey,
  formatRequestedAction,
  parseAction,
  parseSaveSuggestion,
  unwrapNestedJson,
} from "@future-os/thread-projection";
import { describe, expect, it } from "vitest";

describe("parseAction", () => {
  it("uses the explicit escalation trigger for titles with a neutral legacy fallback", () => {
    for (const [trigger, key] of [
      ["model_request", "approval.escalationRequestTitle"],
      ["sandbox_failure", "approval.escalationRetryTitle"],
      [undefined, "approval.escalationTitle"],
      ["unknown", "approval.escalationTitle"],
      [42, "approval.escalationTitle"],
    ] as const) {
      const action = parseAction({
        category: "sandbox_escalation",
        tool: "shell",
        escalation_trigger: trigger,
        // Presence of a model reason must not turn a passive/legacy request active.
        justification: "need access",
      });
      expect(escalationTitleKey(action)).toBe(key);
    }
    expect(escalationTitleKey(null)).toBe("approval.escalationTitle");
    const other = parseAction({
      category: "shell_command",
      tool: "shell",
      escalation_trigger: "model_request",
    });
    expect(other?.escalationTrigger).toBeUndefined();
    expect(escalationTitleKey(other)).toBe("approval.escalationTitle");
  });

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
    expect(
      parseAction(JSON.stringify({ category: "shell_command" })),
    ).toBeNull();
    expect(parseAction(JSON.stringify({ tool: "shell" }))).toBeNull();
    expect(
      parseAction(JSON.stringify({ category: 1, tool: "shell" })),
    ).toBeNull();
  });

  it("parses a minimal valid action", () => {
    expect(
      parseAction(JSON.stringify({ category: "shell_command", tool: "shell" })),
    ).toEqual({
      behavior: undefined,
      blockedPaths: undefined,
      category: "shell_command",
      command: undefined,
      deletes: undefined,
      escalationTrigger: undefined,
      justification: undefined,
      paths: undefined,
      scope: undefined,
      summary: undefined,
      targets: undefined,
      tool: "shell",
      writes: undefined,
    });
  });

  it("drops optional fields with the wrong shape rather than passing them through", () => {
    const action = parseAction(
      JSON.stringify({
        blocked_paths: ["/a", 5],
        category: "file_write",
        command: 123,
        deletes: [{ path: 1 }],
        justification: "",
        paths: "not-an-array",
        scope: {
          cwd: "/w",
          estimatedBlastRadius: "nuclear",
          insideWorkspace: true,
        },
        summary: { nested: true },
        tool: "write",
        writes: [{ path: "/ok", preview: 9 }],
      }),
    );
    expect(action).toEqual({
      behavior: undefined,
      blockedPaths: undefined,
      category: "file_write",
      command: undefined,
      deletes: undefined,
      escalationTrigger: undefined,
      justification: undefined,
      paths: undefined,
      scope: undefined,
      summary: undefined,
      targets: undefined,
      tool: "write",
      writes: undefined,
    });
  });

  it("keeps well-formed optional fields", () => {
    const action = parseAction(
      JSON.stringify({
        blocked_paths: ["/blocked"],
        category: "sandbox_escalation",
        command: "rm -rf /tmp/x",
        deletes: [{ path: "/gone" }],
        justification: "needs it",
        paths: ["/read/a"],
        scope: {
          cwd: "/w",
          estimatedBlastRadius: "high",
          insideWorkspace: false,
        },
        summary: "does a thing",
        tool: "shell",
        writes: [{ path: "/w/a", preview: "hi" }, { path: "/w/b" }],
      }),
    );
    expect(action).toEqual({
      behavior: undefined,
      blockedPaths: ["/blocked"],
      category: "sandbox_escalation",
      command: "rm -rf /tmp/x",
      deletes: [{ path: "/gone" }],
      justification: "needs it",
      paths: ["/read/a"],
      scope: {
        cwd: "/w",
        estimatedBlastRadius: "high",
        insideWorkspace: false,
      },
      summary: "does a thing",
      targets: undefined,
      tool: "shell",
      writes: [{ path: "/w/a", preview: "hi" }, { path: "/w/b" }],
    });
  });

  it("strictly validates Windows capability behavior, scopes, and target count", () => {
    const valid = {
      behavior: "manage_files",
      category: "windows_write_capability",
      command: "build-release",
      targets: [
        { path: "D:\\release", scope: "subtree" },
        { path: "D:\\version.txt", scope: "file" },
      ],
      tool: "shell",
    };
    expect(parseAction(valid)).toMatchObject({
      behavior: "manage_files",
      targets: valid.targets,
    });

    expect(parseAction({ ...valid, behavior: "run_anything" })).toBeNull();
    expect(
      parseAction({
        ...valid,
        targets: [{ path: "D:\\release", scope: "tree" }],
      }),
    ).toBeNull();
    expect(parseAction({ ...valid, targets: [] })).toBeNull();
    expect(
      parseAction({
        ...valid,
        targets: Array.from({ length: 9 }, (_, index) => ({
          path: `D:\\target-${index}`,
          scope: "subtree",
        })),
      }),
    ).toBeNull();
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
    expect(
      parseSaveSuggestion(JSON.stringify({ access: 1, path: "/a" })),
    ).toBeNull();
  });

  it("parses a valid suggestion", () => {
    expect(
      parseSaveSuggestion(JSON.stringify({ access: "write", path: "/w/**" })),
    ).toEqual({
      access: "write",
      path: "/w/**",
    });
  });

  it("parses an atomic multi-target capability suggestion", () => {
    expect(parseSaveSuggestion(JSON.stringify({
      rules: [
        { access: "write", path: "D:\\release" },
        { access: "write", path: "D:\\symbols" },
      ],
    }))).toEqual({
      rules: [
        { access: "write", path: "D:\\release" },
        { access: "write", path: "D:\\symbols" },
      ],
    });
    expect(parseSaveSuggestion(JSON.stringify({ rules: [] }))).toBeNull();
    expect(parseSaveSuggestion(JSON.stringify({
      rules: [{ access: "execute", path: "D:\\bad" }],
    }))).toBeNull();
  });
});

describe("unwrapNestedJson", () => {
  it("returns a non-string value unchanged", () => {
    const obj = { command: "ls" };
    expect(unwrapNestedJson(obj)).toBe(obj);
  });

  it("unwraps single, double, and triple JSON encodings", () => {
    expect(unwrapNestedJson(JSON.stringify({ command: "ls" }))).toEqual({
      command: "ls",
    });
    expect(
      unwrapNestedJson(JSON.stringify(JSON.stringify({ command: "ls" }))),
    ).toEqual({ command: "ls" });
    expect(
      unwrapNestedJson(
        JSON.stringify(JSON.stringify(JSON.stringify({ command: "ls" }))),
      ),
    ).toEqual({ command: "ls" });
  });

  it("stops after maxDepth even if the result is still a string", () => {
    // Four levels of encoding, unwrapped only three times → still a JSON string.
    const quad = JSON.stringify(
      JSON.stringify(JSON.stringify(JSON.stringify("x"))),
    );
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
    expect(formatRequestedAction(JSON.stringify({ command: "echo hi" }))).toBe(
      "echo hi",
    );
    expect(
      formatRequestedAction(
        JSON.stringify(JSON.stringify({ command: "echo hi" })),
      ),
    ).toBe("echo hi");
  });

  it("pretty-prints a JSON object without a command", () => {
    expect(formatRequestedAction(JSON.stringify({ tool: "read" }))).toBe(
      JSON.stringify({ tool: "read" }, null, 2),
    );
  });
});
