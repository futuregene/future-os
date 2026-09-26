import type { Root } from "react-dom/client";
// @vitest-environment jsdom
import type { StoredApprovalRequest } from "../../integrations/storage/types";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { ApprovalPrompt } from "./ApprovalPrompt";

/**
 * The decision surface: every action shape the agent can send, the inline
 * "allow in this workspace" rule editor, the keyboard shortcuts, and the
 * failure paths. `parseAction`/`parseSaveSuggestion` are covered by
 * thread-projection's own tests, so the payloads here are the realistic ones.
 */
const storage = vi.hoisted(() => ({
  saveApprovalRule: vi.fn(async () => {}),
  saveApprovalRules: vi.fn(async () => {}),
}));

vi.mock("../../integrations/storage/runs", () => ({
  saveApprovalRule: storage.saveApprovalRule,
  saveApprovalRules: storage.saveApprovalRules,
}));

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

let root: Root;
let container: HTMLDivElement;

beforeEach(() => {
  vi.clearAllMocks();
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

function approval(over: Partial<StoredApprovalRequest> = {}): StoredApprovalRequest {
  return {
    actionPayload: null,
    id: "ap1",
    kind: "shell_command",
    requestedAction: "shell_command",
    threadId: "T1",
    title: "Run a command",
    ...over,
  } as unknown as StoredApprovalRequest;
}

function render(
  req: StoredApprovalRequest,
  onDecision: (a: StoredApprovalRequest, s: "approved" | "rejected") => Promise<void> = async () => {},
  threadMode?: "chat" | "workspace",
) {
  const decision = vi.fn(onDecision);
  act(() => root.render(
    <ApprovalPrompt approval={req} onDecision={decision} threadMode={threadMode} />,
  ));
  return decision;
}

function button(label: string) {
  return [...container.querySelectorAll<HTMLButtonElement>("button")]
    .find(candidate => candidate.textContent?.trim() === label);
}

function key(init: KeyboardEventInit & { key: string }) {
  act(() => {
    window.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, ...init }));
  });
}

async function click(target: HTMLElement | undefined) {
  await act(async () => {
    target!.click();
    await Promise.resolve();
    await Promise.resolve();
  });
}

it("localizes the title by approval kind and falls back to the agent's own title", () => {
  const titles: [string, string][] = [
    ["file_read", "Approve file read"],
    ["file_write", "Approve file write"],
    ["outside_workspace_write", "Approve outside-workspace file modification"],
    ["shell_command", "Approve shell command"],
    ["something_new", "Run a command"],
  ];
  for (const [kind, expected] of titles) {
    render(approval({ kind, title: "Run a command" }));
    expect(container.querySelector("h2")?.textContent).toBe(expected);
  }
});

it("approves and rejects once, showing the in-flight label", async () => {
  let release: () => void = () => {};
  const decision = render(approval(), async () => {
    await new Promise<void>((resolve) => {
      release = resolve;
    });
  });

  await act(async () => {
    button("Allow once")!.click();
    await Promise.resolve();
  });
  expect(container.textContent).toContain("Allowing");
  expect(decision).toHaveBeenCalledWith(expect.objectContaining({ id: "ap1" }), "approved");

  // concurrency: the in-flight decision disables every control, so a second
  // click cannot start a competing one.
  expect(button("Allowing")!.disabled).toBe(true);
  await click(button("Allowing"));
  expect(decision).toHaveBeenCalledTimes(1);

  await act(async () => {
    release();
    await Promise.resolve();
  });
  expect(container.textContent).toContain("Allow once");
});

it("reports a rejected decision instead of silently staying open", async () => {
  // error-path: the IPC call failed.
  const decision = render(approval(), async () => {
    throw new Error("the run already finished");
  });
  await click(button("Allow once"));

  expect(decision).toHaveBeenCalledTimes(1);
  expect(container.querySelector(".text-danger")?.textContent).toContain("the run already finished");
  // The prompt stays usable for a retry.
  expect(button("Allow once")!.disabled).toBe(false);
});

it("stringifies a non-Error rejection", async () => {
  // error-path: a transport can reject with something that is not an Error (a
  // DOMException-like wrapper), so the message must come from `String(reason)`.
  class TransportFailure {
    toString() {
      return "transport closed";
    }
  }
  render(approval(), async () => {
    throw new TransportFailure();
  });
  await click(button("Allow once"));
  expect(container.querySelector(".text-danger")?.textContent).toContain("transport closed");
});

it("shows the action's shell command and escapes the prompt", async () => {
  render(approval({
    actionPayload: JSON.stringify({ category: "shell_command", command: "rm -rf build", tool: "shell" }),
    kind: "shell_command",
  }));
  expect(container.querySelector("pre")?.textContent).toContain("rm -rf build");

  // Escape rejects the request (the editor is closed).
  key({ key: "Escape" });
  await act(async () => {
    await Promise.resolve();
  });
  expect(container.textContent).toContain("Allow once");
});

it("leaves an unrelated key alone", async () => {
  // boundary: the window listener sees EVERY keystroke while the prompt is up, so
  // any key that is neither Escape nor the paste/Enter accelerator must be inert.
  // Only Escape and Enter were ever dispatched in this file, so the guard's false
  // arm never ran and a stray key - typing, or a shortcut meant for another
  // surface - went untested.
  const decision = render(approval({
    actionPayload: JSON.stringify({ category: "shell_command", command: "ls", tool: "shell" }),
  }));

  key({ key: "a" });
  key({ key: "Tab" });
  key({ key: "Enter" }); // Enter without the platform modifier is not the accelerator
  await act(async () => {
    await Promise.resolve();
  });

  expect(decision).not.toHaveBeenCalled();
  // The prompt is still up and still usable afterwards.
  expect(container.textContent).toContain("Allow once");
  await click(button("Allow once"));
  expect(decision).toHaveBeenCalledWith(expect.objectContaining({ id: "ap1" }), "approved");
});

it("approves on the platform paste/Enter accelerator, but not from a text field", async () => {
  const decision = render(approval({
    actionPayload: JSON.stringify({ category: "shell_command", command: "ls", tool: "shell" }),
  }));

  // boundary: a caret inside an input must keep its own Enter behaviour.
  const input = document.createElement("input");
  document.body.append(input);
  act(() => {
    input.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, ctrlKey: true, key: "Enter" }));
  });
  expect(decision).not.toHaveBeenCalled();
  input.remove();

  key({ ctrlKey: true, key: "Enter" });
  await act(async () => {
    await Promise.resolve();
  });
  expect(decision).toHaveBeenCalledWith(expect.objectContaining({ id: "ap1" }), "approved");

  // The macOS accelerator works too (each test renders its own prompt).
  act(() => root.unmount());
  root = createRoot(container);
  const meta = render(approval());
  key({ key: "Enter", metaKey: true });
  await act(async () => {
    await Promise.resolve();
  });
  expect(meta).toHaveBeenCalledTimes(1);
});

it("lets Escape close the rule editor before it rejects the request", async () => {
  // A two-step Escape: the first closes the editor, the second denies.
  const decision = render(approval({
    actionPayload: JSON.stringify({ category: "file_write", paths: ["a.md"], tool: "edit" }),
    saveSuggestion: JSON.stringify({ access: "write", path: "a.md" }),
  }));

  await click(button("Allow in this workspace"));
  expect(container.querySelector("input")).not.toBeNull();

  key({ key: "Escape" });
  expect(container.querySelector("input")).toBeNull();
  expect(decision).not.toHaveBeenCalled();

  key({ key: "Escape" });
  await act(async () => {
    await Promise.resolve();
  });
  expect(decision).toHaveBeenCalledWith(expect.objectContaining({ id: "ap1" }), "rejected");
});

it("offers the chat wording for a temp-workspace conversation", async () => {
  render(
    approval({
      actionPayload: JSON.stringify({ category: "file_write", paths: ["a.md"], tool: "edit" }),
      saveSuggestion: JSON.stringify({ access: "write", path: "a.md" }),
    }),
    async () => {},
    "chat",
  );
  expect(button("Allow in this chat")).toBeDefined();
});

it("saves an edited workspace rule and then approves once", async () => {
  const decision = render(approval({
    actionPayload: JSON.stringify({ category: "file_write", paths: ["a.md"], tool: "edit" }),
    saveSuggestion: JSON.stringify({ access: "write", path: "a.md" }),
  }));
  await click(button("Allow in this workspace"));

  const input = container.querySelector<HTMLInputElement>("input")!;
  expect(input.value).toBe("a.md");
  await act(async () => {
    const setValue = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!;
    setValue.call(input, "  src/**  ");
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });

  await click(button("Save & allow"));
  expect(storage.saveApprovalRule).toHaveBeenCalledWith({ access: "write", path: "src/**", threadId: "T1" });
  expect(decision).toHaveBeenCalledWith(expect.objectContaining({ id: "ap1" }), "approved");
});

it("refuses an empty rule pattern and keeps the editor open", async () => {
  // boundary: a whitespace-only glob would persist a rule matching everything.
  const decision = render(approval({
    actionPayload: JSON.stringify({ category: "file_write", paths: ["a.md"], tool: "edit" }),
    saveSuggestion: JSON.stringify({ access: "write", path: "a.md" }),
  }));
  await click(button("Allow in this workspace"));

  const input = container.querySelector<HTMLInputElement>("input")!;
  await act(async () => {
    const setValue = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!;
    setValue.call(input, "   ");
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
  await click(button("Save & allow"));

  expect(storage.saveApprovalRule).not.toHaveBeenCalled();
  expect(decision).not.toHaveBeenCalled();
  expect(container.querySelector(".text-danger")?.textContent).toContain("Rule path cannot be empty");
});

it("submits the rule from the input's Enter key and closes on cancel", async () => {
  render(approval({
    actionPayload: JSON.stringify({ category: "file_write", paths: ["a.md"], tool: "edit" }),
    saveSuggestion: JSON.stringify({ access: "write", path: "a.md" }),
  }));
  await click(button("Allow in this workspace"));

  await act(async () => {
    container.querySelector<HTMLInputElement>("input")!.dispatchEvent(
      new KeyboardEvent("keydown", { bubbles: true, key: "Enter" }),
    );
    await Promise.resolve();
  });
  expect(storage.saveApprovalRule).toHaveBeenCalledTimes(1);

  await click(button("Allow in this workspace"));
  await click(button("Cancel"));
  expect(container.querySelector("input")).toBeNull();
});

it("reports a failed rule save without approving", async () => {
  // error-path: writing the rule failed, so the request must stay pending.
  storage.saveApprovalRule.mockRejectedValueOnce(new Error("read-only workspace"));
  const decision = render(approval({
    actionPayload: JSON.stringify({ category: "file_write", paths: ["a.md"], tool: "edit" }),
    saveSuggestion: JSON.stringify({ access: "write", path: "a.md" }),
  }));
  await click(button("Allow in this workspace"));
  await click(button("Save & allow"));

  expect(decision).not.toHaveBeenCalled();
  expect(container.querySelector(".text-danger")?.textContent).toContain("read-only workspace");
  expect(container.querySelector("input")).not.toBeNull();
});

it("saves every capability rule from the suggestion, then approves", async () => {
  const rules = [{ access: "write", path: "C:/work/**" }];
  const decision = render(approval({
    actionPayload: JSON.stringify({
      behavior: "manage_files",
      category: "windows_write_capability",
      targets: [{ path: "C:/work/a.txt", scope: "file" }],
      tool: "edit",
    }),
    kind: "windows_write_capability",
    saveSuggestion: JSON.stringify({ rules }),
  }));

  // The capability prompt names the writable paths, not the generic action.
  expect(container.querySelector("h2")?.textContent).toContain("Approve file modification");
  await click(button("Allow in this workspace"));

  expect(storage.saveApprovalRules).toHaveBeenCalledWith({ rules, threadId: "T1" });
  expect(decision).toHaveBeenCalledTimes(1);
});

it("reports a failed capability-rule save", async () => {
  storage.saveApprovalRules.mockRejectedValueOnce(new Error("permission denied"));
  const decision = render(approval({
    actionPayload: JSON.stringify({
      behavior: "manage_files",
      category: "windows_write_capability",
      targets: [{ path: "C:/work/a.txt", scope: "file" }],
      tool: "edit",
    }),
    kind: "windows_write_capability",
    saveSuggestion: JSON.stringify({ rules: [{ access: "write", path: "C:/work/**" }] }),
  }));
  await click(button("Allow in this workspace"));

  expect(decision).not.toHaveBeenCalled();
  expect(container.querySelector(".text-danger")?.textContent).toContain("permission denied");
});

it("stringifies a non-Error rejection from the rule-save path too", async () => {
  // error-path: `confirmRule` saves the rule and THEN asks for the decision, so
  // the transport can reject after the write succeeded. This is a different
  // `instanceof Error` arm from the one the existing "stringifies a non-Error
  // rejection" test covers - that one goes through `decide` (the plain allow
  // button); this one goes through `confirmRule`. A branch audit found the arm
  // at zero hits despite the similarly-named test existing, which is how a test
  // name can look like coverage it does not give.
  class TransportFailure {
    toString() {
      return "transport closed after save";
    }
  }
  const decision = render(
    approval({
      actionPayload: JSON.stringify({ category: "file_write", paths: ["a.md"], tool: "edit" }),
      saveSuggestion: JSON.stringify({ access: "write", path: "a.md" }),
    }),
    async () => {
      throw new TransportFailure();
    },
  );
  await click(button("Allow in this workspace"));
  await click(button("Save & allow"));

  // The rule was written...
  expect(storage.saveApprovalRule).toHaveBeenCalledTimes(1);
  // ...and the failure is reported from `String(reason)`, not swallowed.
  expect(container.querySelector(".text-danger")?.textContent).toContain("transport closed after save");
  expect(decision).toHaveBeenCalledTimes(1);
});

it("stringifies a non-Error rejection from the capability-rule path too", async () => {
  // error-path: the same arm in `confirmCapabilityRules`, which writes every
  // rule before asking for the decision.
  class TransportFailure {
    toString() {
      return "transport closed after rules";
    }
  }
  render(
    approval({
      actionPayload: JSON.stringify({
        behavior: "manage_files",
        category: "windows_write_capability",
        targets: [{ path: "C:/work/a.txt", scope: "file" }],
        tool: "edit",
      }),
      kind: "windows_write_capability",
      saveSuggestion: JSON.stringify({ rules: [{ access: "write", path: "C:/work/**" }] }),
    }),
    async () => {
      throw new TransportFailure();
    },
  );
  await click(button("Allow in this workspace"));

  expect(storage.saveApprovalRules).toHaveBeenCalledTimes(1);
  expect(container.querySelector(".text-danger")?.textContent).toContain("transport closed after rules");
});

it("warns when a capability request arrives without a parsable payload", async () => {
  // error-path: the agent sent a capability approval we cannot describe, so the
  // buttons must be inert rather than approving something unknown.
  render(approval({
    actionPayload: "not json",
    kind: "windows_write_capability",
    saveSuggestion: JSON.stringify({ rules: [{ access: "write", path: "C:/work/**" }] }),
  }));

  expect(container.textContent).toContain("incomplete and cannot be safely approved");
  expect(button("Allow once")!.disabled).toBe(true);
  expect(button("Allow in this workspace")!.disabled).toBe(true);
  expect(button("Deny")!.disabled).toBe(false);
});

it("renders a capability action with its command and per-target scope warning", () => {
  render(approval({
    actionPayload: JSON.stringify({
      behavior: "manage_files",
      category: "windows_write_capability",
      command: "xcopy /E src dst",
      targets: [
        { path: "C:/work/file.txt", scope: "file" },
        { path: "C:/work/tree", scope: "subtree" },
      ],
      tool: "edit",
    }),
    kind: "windows_write_capability",
  }));

  expect(container.querySelector("pre")?.textContent).toContain("xcopy /E src dst");
  expect(container.textContent).toContain("C:/work/file.txt");
  // boundary: only the subtree target carries the "everything below this" glyph.
  const warnings = container.querySelectorAll("[aria-label]");
  expect(warnings).toHaveLength(1);
});

it("renders an escalation's justification, blocked paths and unsandboxed note", () => {
  render(approval({
    actionPayload: JSON.stringify({
      blocked_paths: ["/etc/passwd"],
      category: "sandbox_escalation",
      command: "chmod 777 /etc/passwd",
      escalation_trigger: "sandbox_failure",
      justification: "the sandbox refused the write",
      paths: ["/etc/passwd"],
      tool: "shell",
    }),
  }));

  expect(container.textContent).toContain("chmod 777 /etc/passwd");
  expect(container.textContent).toContain("the sandbox refused the write");
  expect(container.textContent).toContain("/etc/passwd");
  // The escalation is not sandboxed, and the user must be told.
  expect(container.textContent?.toLowerCase()).toContain("without sandbox restrictions");
});

it("titles an escalation by its trigger when the approval kind is an escalation", async () => {
  // boundary: the title branch keys off `approval.kind`, NOT the payload's
  // `category`. The neighbouring test sets `category: "sandbox_escalation"` but
  // leaves `kind` at its default `shell_command`, so it never enters this arm -
  // a branch audit found the arm at zero hits while that test passed. Both
  // trigger variants are asserted because they select different titles.
  render(approval({
    actionPayload: JSON.stringify({ category: "sandbox_escalation", escalation_trigger: "model_request", tool: "shell" }),
    kind: "sandbox_escalation",
  }));
  expect(container.querySelector("h2")?.textContent).toContain("The model requests to run this command outside the sandbox");

  render(approval({
    actionPayload: JSON.stringify({ category: "sandbox_escalation", escalation_trigger: "sandbox_failure", tool: "shell" }),
    kind: "sandbox_escalation",
  }));
  expect(container.querySelector("h2")?.textContent).toContain("This command needs to run outside the sandbox");
});

it("lets a keyboard event from a select through to the select", async () => {
  // boundary: `isEditableTarget` treats a `<select>` as a form control, so the
  // global shortcut handler must not act on keys typed into one. The arm was
  // uncovered because no test ever put a select inside the prompt.
  render(approval());
  const select = document.createElement("select");
  container.append(select);

  const event = new KeyboardEvent("keydown", { bubbles: true, cancelable: true, key: "Enter" });
  select.dispatchEvent(event);
  // Not swallowed by the prompt's own handler...
  expect(event.defaultPrevented).toBe(false);
});

it("omits the escalation notes for an ordinary shell command", () => {
  render(approval({
    actionPayload: JSON.stringify({ category: "shell_command", command: "ls -la", tool: "shell" }),
  }));
  expect(container.textContent).toContain("ls -la");
  expect(container.textContent?.toLowerCase()).not.toContain("without sandbox restrictions");
});

it("renders file writes with their previews", () => {
  render(approval({
    actionPayload: JSON.stringify({
      category: "file_write",
      tool: "write",
      writes: [{ path: "src/a.ts", preview: "export const a = 1;" }, { path: "src/b.ts" }],
    }),
    kind: "file_write",
  }));

  expect(container.textContent).toContain("src/a.ts");
  expect(container.textContent).toContain("export const a = 1;");
  // boundary: a write with no preview still gets its row.
  expect(container.textContent).toContain("src/b.ts");
});

it("renders a read's paths, or the bare summary when it has no paths", () => {
  render(approval({
    actionPayload: JSON.stringify({ category: "file_read", paths: ["a.md", "b.md"], tool: "read" }),
    kind: "file_read",
  }));
  expect(container.textContent).toContain("a.md");
  expect(container.textContent).toContain("b.md");

  act(() => root.unmount());
  root = createRoot(container);
  render(approval({
    actionPayload: JSON.stringify({ category: "file_read", summary: "read two files", tool: "read" }),
    kind: "file_read",
  }));
  expect(container.textContent).toContain("read two files");
});

it("renders nothing extra for an action with no describable content", () => {
  render(approval({ actionPayload: JSON.stringify({ category: "file_read", tool: "read" }) }));
  expect(container.querySelector("pre")).toBeNull();
  expect(container.querySelector("ul")).toBeNull();
});

it("falls back to the raw requested action when the payload cannot be parsed", () => {
  // boundary: an unparsable payload still has to tell the approver what happens.
  render(approval({ actionPayload: "not json", requestedAction: "shell_command" }));
  expect(container.querySelector("pre")?.textContent).toContain("shell_command");
});

it("strips the Windows extended-length prefix from a displayed path", () => {
  // platform-cfg: the agent reports `\\?\C:\...` internally; an approver must see
  // the ordinary spelling, including the UNC form.
  render(approval({
    actionPayload: JSON.stringify({
      category: "file_read",
      paths: ["\\\\?\\C:\\work\\a.txt", "\\\\?\\UNC\\server\\share\\b.txt", "C:\\plain.txt"],
      tool: "read",
    }),
    kind: "file_read",
  }));

  expect(container.textContent).toContain("C:\\work\\a.txt");
  expect(container.textContent).toContain("\\\\server\\share\\b.txt");
  expect(container.textContent).toContain("C:\\plain.txt");
  expect(container.textContent).not.toContain("?\\");
});

it("ignores an unparsable save suggestion instead of opening an empty editor", async () => {
  // boundary: a suggestion whose glob is the empty string parses as a
  // suggestion (the field is a string) but carries no glob to edit, so nothing
  // must open rather than an editor pre-filled with "".
  const decision = render(approval({
    actionPayload: JSON.stringify({ category: "file_write", paths: ["a.md"], tool: "edit" }),
    saveSuggestion: JSON.stringify({ access: "write", path: "" }),
  }));

  await click(button("Allow in this workspace"));

  expect(container.querySelector("input")).toBeNull();
  expect(decision).not.toHaveBeenCalled();
  expect(container.querySelector(".text-danger")).toBeNull();
});

it("ignores a second rule submit while the first is still saving", async () => {
  // concurrency: the pattern input stays enabled during the save, so a second
  // Enter must not persist the rule (or approve) twice.
  let release: () => void = () => {};
  storage.saveApprovalRule.mockImplementationOnce(() => new Promise<void>((resolve) => {
    release = resolve;
  }));
  const decision = render(approval({
    actionPayload: JSON.stringify({ category: "file_write", paths: ["a.md"], tool: "edit" }),
    saveSuggestion: JSON.stringify({ access: "write", path: "a.md" }),
  }));
  await click(button("Allow in this workspace"));

  const input = container.querySelector<HTMLInputElement>("input")!;
  await act(async () => {
    input.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, key: "Enter" }));
    await Promise.resolve();
  });
  expect(storage.saveApprovalRule).toHaveBeenCalledTimes(1);

  // A second submit while `deciding` is set is dropped by the guard.
  await act(async () => {
    input.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, key: "Enter" }));
    await Promise.resolve();
  });
  expect(storage.saveApprovalRule).toHaveBeenCalledTimes(1);

  await act(async () => {
    release();
    await Promise.resolve();
  });
  expect(decision).toHaveBeenCalledTimes(1);
});

it("denies the request from the deny button", async () => {
  const decision = render(approval({
    actionPayload: JSON.stringify({ category: "shell_command", command: "ls", tool: "shell" }),
  }));
  await click(button("Deny"));
  expect(decision).toHaveBeenCalledWith(expect.objectContaining({ id: "ap1" }), "rejected");
});

it("ignores the keyboard while a decision is already in flight", async () => {
  // concurrency: the global listener is live during the in-flight decision, so
  // a second accelerator press must not queue a competing decision.
  let release: () => void = () => {};
  const decision = render(approval({ requestedAction: "shell_command" }), async () => {
    await new Promise<void>((resolve) => {
      release = resolve;
    });
  });

  await act(async () => {
    button("Allow once")!.click();
    await Promise.resolve();
  });
  expect(decision).toHaveBeenCalledTimes(1);

  key({ key: "Escape" });
  key({ ctrlKey: true, key: "Enter" });
  await act(async () => {
    await Promise.resolve();
  });
  expect(decision).toHaveBeenCalledTimes(1);

  await act(async () => {
    release();
    await Promise.resolve();
  });
});

it("keeps the caret's own Escape in a text field", () => {
  // boundary: Escape inside an input belongs to the field, not to a rejection.
  const decision = render(approval({
    actionPayload: JSON.stringify({ category: "file_write", paths: ["a.md"], tool: "edit" }),
    saveSuggestion: JSON.stringify({ access: "write", path: "a.md" }),
  }));

  const textarea = document.createElement("textarea");
  document.body.append(textarea);
  act(() => {
    textarea.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, key: "Escape" }));
  });
  expect(decision).not.toHaveBeenCalled();
  textarea.remove();
});
