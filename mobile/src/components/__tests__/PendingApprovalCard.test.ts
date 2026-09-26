import { createElement } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { Button } from "../Button";
import { PendingApprovalCard } from "../TimelineCard";
import type { ApprovalPayload } from "../../remote/types";
import "../../i18n";

// The card renders real copy from the shipped deck: asserting on the English
// strings is what proves a key exists and interpolates, which a bare-key mock
// would hide. MarkdownText is irrelevant here but heavy to transform.
jest.mock("../MarkdownText", () => ({ MarkdownText: "MarkdownText" }));
jest.mock("lucide-react-native", () => Object.fromEntries(
  ["AlertTriangle", "Brain", "Check", "ChevronDown", "ChevronUp", "CircleAlert", "Copy",
    "FileText", "Paperclip", "Pencil", "TerminalSquare", "TriangleAlert", "Wrench", "X"]
    .map(name => [name, name]),
));

let tree: ReactTestRenderer;
afterEach(() => { if (tree) act(() => tree.unmount()); });

function render(payload: ApprovalPayload, extra: Partial<Parameters<typeof PendingApprovalCard>[0]> = {}) {
  const onDecision = jest.fn();
  act(() => {
    tree = create(createElement(PendingApprovalCard, {
      payload, submitting: false, error: null, onDecision, ...extra,
    }));
  });
  return { onDecision };
}

/** Host text nodes: a string child can belong to a composite and its host, so
 * count only host instances (a value painted once counts once). */
function painted(text: string) {
  return tree.root.findAll(
    node => typeof node.type === "string" && node.props.children === text,
  ).length;
}
function hasText(text: string) {
  return painted(text) > 0;
}
function buttons() {
  return tree.root.findAllByType(Button).map(button => ({
    label: button.props.label as string,
    disabled: button.props.disabled as boolean,
    loading: button.props.loading as boolean,
  }));
}
/** The innermost tappable row whose subtree paints exactly `text` (a labelled
 * button resolves to itself; the command toggle has no label of its own). */
function press(text: string) {
  const node = tree.root.findAll(
    candidate => typeof candidate.props.onPress === "function"
      && candidate.findAll(inner => inner.props.children === text).length > 0,
  ).at(-1);
  expect(node).toBeDefined();
  act(() => node!.props.onPress());
}

const fileWrite = (paths: string[]): ApprovalPayload => ({
  approval_request_id: "req-1",
  kind: "file_write",
  action: { tool: "write", category: "file_write", paths },
});
const fileRead = (paths: string[]): ApprovalPayload => ({
  approval_request_id: "req-2",
  kind: "file_read",
  action: { tool: "read", category: "file_read", paths },
});
const capability = (
  targets: { path: string; scope: "file" | "subtree" }[],
  command?: string,
): ApprovalPayload => ({
  approval_request_id: "req-3",
  kind: "windows_write_capability",
  action: { tool: "write", category: "windows_write_capability", behavior: "manage_files", targets, command },
});

describe("the approval card shows what the decision is about", () => {
  test("a single-file write names the file, and each button reports its decision", () => {
    const { onDecision } = render(fileWrite(["/workspace/src/app.ts"]));
    expect(hasText("Approve file write")).toBe(true);
    expect(hasText("Agent wants to modify a protected file.")).toBe(true);
    expect(hasText("Write file")).toBe(true);
    expect(hasText("/workspace/src/app.ts")).toBe(true);
    expect(buttons()).toEqual([
      { label: "Deny", disabled: false, loading: false },
      { label: "Allow once", disabled: false, loading: false },
    ]);
    press("Allow once");
    expect(onDecision).toHaveBeenCalledWith("approved");
    press("Deny");
    expect(onDecision).toHaveBeenCalledWith("rejected");
    expect(onDecision).toHaveBeenCalledTimes(2);
  });

  test("a multi-file write counts the files and paints every path", () => {
    render(fileWrite(["/a/one.ts", "/a/两个.ts", `/a/${"deep/".repeat(60)}long.ts`]));
    expect(hasText("Write 3 files")).toBe(true);
    expect(hasText("/a/one.ts")).toBe(true);
    expect(hasText("/a/两个.ts")).toBe(true);
    expect(hasText(`/a/${"deep/".repeat(60)}long.ts`)).toBe(true);
  });

  test("a read is labelled as a read, never as a write", () => {
    render(fileRead(["/etc/hosts", "/etc/passwd"]));
    expect(hasText("Approve file read")).toBe(true);
    expect(hasText("Approve file write")).toBe(false);
    expect(hasText("Read file")).toBe(true);
    // Read paths are capped at two lines so a long path cannot grow the dock
    // (the title is capped too, but it is not selectable like a path).
    const pathRows = tree.root.findAll(node =>
      typeof node.type === "string" && node.props.selectable === true && node.props.numberOfLines === 2);
    expect(pathRows.map(row => row.props.children)).toEqual(["/etc/hosts", "/etc/passwd"]);
  });

  test("deletes are what the user judges, and shadow the paths behind them", () => {
    render({
      approval_request_id: "req-d",
      kind: "file_write",
      action: {
        tool: "write", category: "file_write",
        paths: ["/keep/untouched.ts"],
        deletes: [{ path: "/gone/one.ts" }],
      },
    });
    expect(hasText("Delete file")).toBe(true);
    expect(hasText("/gone/one.ts")).toBe(true);
    expect(hasText("Delete 1 files")).toBe(false);
    expect(hasText("/keep/untouched.ts")).toBe(false);
  });

  test("several deletes are counted", () => {
    render({
      approval_request_id: "req-d2",
      kind: "file_write",
      action: {
        tool: "write", category: "file_write",
        deletes: [{ path: "/gone/a" }, { path: "/gone/b" }, { path: "/gone/c" }],
      },
    });
    expect(hasText("Delete 3 files")).toBe(true);
    expect(hasText("/gone/b")).toBe(true);
  });

  test("a shell command shows the command instead of a path list", () => {
    render({
      approval_request_id: "req-sh",
      kind: "shell_command",
      action: { tool: "shell", category: "shell_command", command: "rm -rf ./build && npm run build" },
    });
    expect(hasText("Approve shell command")).toBe(true);
    expect(hasText("Shell command")).toBe(true);
    expect(hasText("rm -rf ./build && npm run build")).toBe(true);
  });
});

describe("unknown and escalation payloads keep a usable title", () => {
  test("an unknown kind falls back to the wire title, then the tool name, then the generic title", () => {
    render({ approval_request_id: "u1", kind: "future_kind", title: "Wire title", summary: "Wire summary" });
    expect(hasText("Wire title")).toBe(true);
    expect(hasText("Wire summary")).toBe(true);
    act(() => tree.unmount());
    render({ approval_request_id: "u2", kind: "future_kind", tool_name: "some_tool" });
    expect(hasText("some_tool")).toBe(true);
    act(() => tree.unmount());
    render({ approval_request_id: "u3" });
    expect(hasText("Approval required")).toBe(true);
    expect(buttons().map(button => button.label)).toEqual(["Deny", "Allow once"]);
  });

  test.each([
    ["model_request", "The model requests to run this command outside the sandbox"],
    ["sandbox_failure", "This command needs to run outside the sandbox"],
    ["", "Run this command outside the sandbox"],
  ])("a sandbox escalation with trigger %s titles the request", (trigger, expected) => {
    render({
      approval_request_id: "esc",
      kind: "sandbox_escalation",
      action: {
        tool: "shell",
        category: "sandbox_escalation",
        command: "sudo apt install ffmpeg",
        escalation_trigger: trigger || undefined,
      },
    });
    expect(hasText(expected)).toBe(true);
    expect(hasText("sudo apt install ffmpeg")).toBe(true);
    expect(hasText("Shell command")).toBe(true);
  });

  test("a payload with no action renders the title alone", () => {
    render({ approval_request_id: "no-action", kind: "file_write" });
    expect(hasText("Approve file write")).toBe(true);
    expect(hasText("Write file")).toBe(false);
    expect(hasText("Shell command")).toBe(false);
  });
});

describe("the Windows write capability", () => {
  test("a single file target names that file and folds the command behind a toggle", () => {
    const { onDecision } = render(capability([{ path: "C:\\Users\\me\\报告.docx", scope: "file" }], "python build.py --out 报告.docx"));
    expect(hasText("Allow FutureOS to modify C:\\Users\\me\\报告.docx?")).toBe(true);
    // A single named target needs no separate "Write file" label above it.
    expect(hasText("Write file")).toBe(false);
    expect(hasText("View command")).toBe(true);
    // The command is hidden until asked for: the decision is about the target.
    expect(hasText("python build.py --out 报告.docx")).toBe(false);
    press("View command");
    expect(hasText("python build.py --out 报告.docx")).toBe(true);
    press("View command");
    expect(hasText("python build.py --out 报告.docx")).toBe(false);
    press("Allow once");
    expect(onDecision).toHaveBeenCalledWith("approved");
  });

  test("a subtree target is described as managing files in a directory", () => {
    render(capability([{ path: "D:\\repo\\src", scope: "subtree" }]));
    expect(hasText("Allow FutureOS to manage files in D:\\repo\\src?")).toBe(true);
    expect(hasText("Allow FutureOS to modify D:\\repo\\src?")).toBe(false);
  });

  test("several targets are counted and listed under one Locations label", () => {
    render(capability([
      { path: "D:\\repo\\a", scope: "file" },
      { path: "D:\\repo\\b", scope: "subtree" },
      { path: "D:\\repo\\c", scope: "file" },
    ]));
    expect(hasText("Allow FutureOS to manage files in these 3 locations?")).toBe(true);
    expect(hasText("Locations")).toBe(true);
    for (const path of ["D:\\repo\\a", "D:\\repo\\b", "D:\\repo\\c"]) {
      expect(hasText(path)).toBe(true);
    }
  });

  test("eight targets are the documented maximum and still render", () => {
    render(capability(Array.from({ length: 8 }, (_value, index) => ({
      path: `D:\\repo\\p${index}`, scope: "file" as const,
    }))));
    expect(hasText("Allow FutureOS to manage files in these 8 locations?")).toBe(true);
    expect(hasText("D:\\repo\\p7")).toBe(true);
  });

  test.each([
    ["more targets than the wire allows", Array.from({ length: 9 }, (_v, i) => ({ path: `D:\\p${i}`, scope: "file" as const }))],
    ["an empty target list", [] as { path: string; scope: "file" }[]],
    ["a target with no path", [{ path: "", scope: "file" as const }]],
    ["a target without a scope", [{ path: "D:\\x" } as unknown as { path: string; scope: "file" }]],
  ])("a capability request with %s is refused, not approved", (_case, targets) => {
    const { onDecision } = render({
      approval_request_id: "bad-cap",
      kind: "windows_write_capability",
      action: { tool: "write", category: "windows_write_capability", behavior: "manage_files", targets },
    });
    expect(hasText("This approval request is incomplete and cannot be safely approved.")).toBe(true);
    expect(buttons()).toEqual([
      { label: "Deny", disabled: false, loading: false },
      { label: "Allow once", disabled: true, loading: false },
    ]);
    // Rejection stays available: an unapprovable request must still be closable.
    press("Deny");
    expect(onDecision).toHaveBeenCalledWith("rejected");
  });

  test("a capability payload that also carries a malformed write shows both the reason and the failure", () => {
    render(
      {
        approval_request_id: "bad-cap-2",
        kind: "windows_write_capability",
        action: { tool: "write", category: "windows_write_capability" },
      },
      { error: "Could not submit the approval decision." },
    );
    expect(hasText("This approval request is incomplete and cannot be safely approved.")).toBe(true);
    expect(hasText("Could not submit the approval decision.")).toBe(true);
  });

  test("a JSON-string action is accepted like the parsed object some bridges send", () => {
    render({
      approval_request_id: "string-action",
      kind: "windows_write_capability",
      action: JSON.stringify({
        tool: "write",
        category: "windows_write_capability",
        behavior: "manage_files",
        targets: [{ path: "C:\\a", scope: "subtree" }],
      }),
    });
    expect(hasText("Allow FutureOS to manage files in C:\\a?")).toBe(true);
  });
});

describe("the wait and the failure are visible on the card itself", () => {
  test("while a decision is in flight both buttons are locked and spell out the wait", () => {
    render(fileWrite(["/a/one.ts"]), { submitting: true });
    expect(buttons()).toEqual([
      { label: "Denying", disabled: true, loading: true },
      { label: "Allowing", disabled: true, loading: true },
    ]);
  });

  test("a decision that failed says so without unlocking an unapprovable request", () => {
    const { onDecision } = render(fileWrite(["/a/one.ts"]), { error: "Could not submit the approval decision." });
    expect(hasText("Could not submit the approval decision.")).toBe(true);
    // A retryable failure leaves both decisions available.
    expect(buttons().every(button => !button.disabled)).toBe(true);
    press("Allow once");
    expect(onDecision).toHaveBeenCalledWith("approved");
  });
});
