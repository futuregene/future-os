import { describe, expect, it } from "vitest";
import { numberOrStringField, parseJsonish, recordOf, stringField, toolCommand } from "./toolInput";

describe("toolInput", () => {
  it("toolCommand reads a plain JSON object", () => {
    expect(toolCommand("{\"command\":\"ls -la\"}")).toBe("ls -la");
  });

  it("toolCommand unwraps a double-encoded payload", () => {
    const doubled = JSON.stringify(JSON.stringify({ command: "git status" }));
    expect(toolCommand(doubled)).toBe("git status");
  });

  it("toolCommand returns null for a non-JSON string", () => {
    expect(toolCommand("not json at all")).toBeNull();
  });

  it("toolCommand returns null for empty / nullish input", () => {
    expect(toolCommand("")).toBeNull();
    expect(toolCommand("   ")).toBeNull();
    expect(toolCommand(null)).toBeNull();
    expect(toolCommand(undefined)).toBeNull();
  });

  it("recordOf narrows objects and rejects arrays / scalars", () => {
    expect(recordOf("{\"a\":1}")).toEqual({ a: 1 });
    expect(recordOf("[1,2,3]")).toBeNull();
    expect(recordOf("\"plain\"")).toBeNull();
    expect(recordOf("42")).toBeNull();
  });

  it("parseJsonish keeps a non-JSON string as-is but empties whitespace", () => {
    expect(parseJsonish("hello")).toBe("hello");
    expect(parseJsonish("   ")).toBeNull();
  });

  it("numberOrStringField surfaces a numeric field (incl. 0)", () => {
    const record = { exit_code: 0, status: "ok" };
    expect(numberOrStringField(record, ["exitStatus", "exit_code"])).toBe("0");
    expect(stringField(record, ["status"])).toBe("ok");
  });

  it("stringField accepts a single key or an array and skips blanks", () => {
    expect(stringField({ command: "echo" }, "command")).toBe("echo");
    expect(stringField({ command: "  " }, "command")).toBeNull();
    expect(stringField({ cmd: "echo" }, ["command", "cmd"])).toBe("echo");
    expect(stringField(null, "command")).toBeNull();
  });
});
