// @vitest-environment jsdom
import type { AgentActivityItem, AgentActivityKind } from "@future-os/thread-projection";
import type { Root } from "react-dom/client";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, it } from "vitest";
import { onFutureEvent } from "../../lib/futureEvents";
import { AgentActivityLine, AgentActivityList } from "./AgentActivityList";

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

let root: Root;
let container: HTMLDivElement;

beforeEach(() => {
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

function item(over: Partial<AgentActivityItem> = {}): AgentActivityItem {
  return { id: "a1", kind: "shell", status: "completed", ...over } as AgentActivityItem;
}

function render(element: Parameters<Root["render"]>[0]) {
  act(() => root.render(element));
}

/** The wrapper `AgentActivityList` renders around its lines. */
function list() {
  return container.firstElementChild as HTMLElement | null;
}

function buttons() {
  return [...container.querySelectorAll<HTMLButtonElement>("button")];
}

async function clickButton(index = 0) {
  await act(async () => {
    buttons()[index]!.click();
    await Promise.resolve();
  });
}

it("renders nothing without items, with an empty list, or with no visible status", () => {
  render(<AgentActivityList />);
  expect(container.firstElementChild).toBeNull();

  render(<AgentActivityList items={[]} />);
  expect(container.firstElementChild).toBeNull();

  // boundary: `pending` calls are not user-visible activity yet.
  render(<AgentActivityList items={[item({ status: "pending" } as unknown as Partial<AgentActivityItem>)]} />);
  expect(container.firstElementChild).toBeNull();

  render(<AgentActivityList items={[item({ status: "cancelled" } as unknown as Partial<AgentActivityItem>)]} />);
  expect(container.firstElementChild).toBeNull();
});

it("keeps only running, completed and failed calls, in order", () => {
  render(
    <AgentActivityList items={[
      item({ id: "r", status: "running" }),
      item({ id: "p", kind: "read", status: "pending" } as unknown as Partial<AgentActivityItem>),
      item({ id: "c", kind: "edit", status: "completed" }),
      item({ id: "f", kind: "write", status: "failed" }),
    ]}
    />,
  );

  const lines = [...list()!.children] as HTMLElement[];
  expect(lines).toHaveLength(3);
  expect(lines.map(line => line.textContent)).toEqual([
    "Running a command",
    "Edited a file",
    "Write failed",
  ]);
});

it("labels a single call from its kind and status", () => {
  const labels: [AgentActivityKind, string, string, string][] = [
    // [kind, running label, completed label, failed label]
    ["read", "Reading a file", "Read a file", "Read failed"],
    ["shell", "Running a command", "Ran a command", "Command failed"],
    ["write", "Writing a file", "Wrote a file", "Write failed"],
    ["edit", "Editing a file", "Edited a file", "Edit failed"],
  ];

  for (const [kind, running, completed, failed] of labels) {
    render(<AgentActivityLine item={item({ kind, status: "running" })} />);
    expect(container.textContent).toBe(running);

    render(<AgentActivityLine item={item({ kind, status: "completed" })} />);
    expect(container.textContent).toBe(completed);

    render(<AgentActivityLine item={item({ kind, status: "failed" })} />);
    expect(container.textContent).toBe(failed);
  }

  // Thinking is the one status-independent label.
  render(<AgentActivityLine item={item({ kind: "thinking", status: "running" })} />);
  expect(container.textContent).toBe("Thinking");
});

it("labels a burst by the tool kind it grouped", () => {
  const cases: [AgentActivityKind, number, string][] = [
    ["shell", 2, "Ran 2 commands"],
    ["shell", 7, "Ran 7 commands"],
    ["write", 3, "Wrote 3 files"],
    ["read", 4, "Read 4 files"],
    ["edit", 5, "Edited 5 files"],
  ];

  for (const [kind, count, label] of cases) {
    render(<AgentActivityLine item={item({ children: [item()], count, kind, status: "completed" })} />);
    expect(container.textContent).toBe(label);
  }
});

it("treats a one-child burst as a single call, never as a plural", () => {
  // boundary: a group is only formed for a real burst, so `count: 1` must not
  // produce "Ran 1 command" — it falls through to the plain single-call label.
  render(<AgentActivityLine item={item({ children: [item()], count: 1, kind: "read" })} />);
  expect(container.textContent).toBe("Read a file");
});

it("shows the alert glyph for a failed call of every kind, and a pulse while thinking", () => {
  for (const kind of ["shell", "read", "write", "edit"] as AgentActivityKind[]) {
    render(<AgentActivityLine item={item({ kind, status: "failed" })} />);
    expect(container.querySelector("svg")).not.toBeNull();
    // A failed call is flagged with a single glyph, never a label suffix.
    expect(container.textContent).toBe(
      ({
        edit: "Edit failed",
        read: "Read failed",
        shell: "Command failed",
        write: "Write failed",
      } as Record<string, string>)[kind],
    );
  }

  render(<AgentActivityLine item={item({ kind: "thinking", status: "running" })} />);
  expect(container.querySelector("svg.animate-pulse")).not.toBeNull();

  // Not running → no infinite animation on an already-finished thought.
  render(<AgentActivityLine item={item({ kind: "thinking", status: "completed" })} />);
  expect(container.querySelector("svg.animate-pulse")).toBeNull();
});

it("expands a standalone call into its target and back", async () => {
  render(<AgentActivityLine item={item({ kind: "read", target: "src/a.ts" })} workspacePath={null} />);
  expect(container.textContent).toBe("Read a file");

  await clickButton();
  expect(buttons()[0]!.getAttribute("aria-expanded")).toBe("true");
  expect(container.textContent).toContain("src/a.ts");

  await clickButton();
  expect(buttons()[0]!.getAttribute("aria-expanded")).toBe("false");
  expect(container.textContent).not.toContain("src/a.ts");
});

it("relativizes a file target against the workspace but leaves a command alone", () => {
  render(<AgentActivityLine item={item({ kind: "read", target: "/work/proj/src/a.ts" })} workspacePath="/work/proj" />);
  expect(container.textContent).toBe("Read a file");

  render(
    <AgentActivityLine
      item={item({ kind: "shell", target: "npm test --silent" })}
      workspacePath="/work/proj"
    />,
  );
  // A shell target is the command itself; relativizing it would corrupt it.
  expect(container.textContent).toBe("Ran a command");
});

it("emits inspect-tool with the run and call ids", async () => {
  const events: { runId: string; toolId: string }[] = [];
  const off = onFutureEvent("inspect-tool", detail => void events.push(detail));

  render(<AgentActivityLine item={item({ id: "call-1" })} runId="R1" />);
  await clickButton();
  expect(events).toEqual([{ runId: "R1", toolId: "call-1" }]);
  off();
});

it("reveals a target without inspecting when no run is known", async () => {
  // boundary: the same toggle is offered for the target alone; with no run there
  // is nothing to inspect, so only the reveal may happen.
  const events: unknown[] = [];
  const off = onFutureEvent("inspect-tool", detail => void events.push(detail));

  render(<AgentActivityLine item={item({ kind: "read", target: "src/a.ts" })} runId={null} />);
  await clickButton();

  expect(events).toEqual([]);
  expect(container.textContent).toContain("src/a.ts");
  off();
});

it("renders a call with neither a run nor a target as plain text", () => {
  // boundary: nothing to inspect and nothing to reveal → no interactive control.
  render(<AgentActivityLine item={item({ target: null as unknown as string })} runId={null} />);
  expect(buttons()).toHaveLength(0);
  expect(container.textContent).toBe("Ran a command");
});

it("shows an edit's line counts and omits the half that is missing", () => {
  render(<AgentActivityLine item={item({ additions: 12, deletions: 3, kind: "edit" })} />);
  expect(container.textContent).toContain("+12 -3");

  // boundary: a brand-new file has additions only.
  render(<AgentActivityLine item={item({ additions: 7, kind: "write" })} />);
  expect(container.textContent).toContain("+7");
  expect(container.textContent).not.toContain("-");

  // boundary: a pure edit may report 0 additions with deletions.
  render(<AgentActivityLine item={item({ deletions: 0, kind: "edit" })} />);
  expect(container.textContent).toContain("-0");
});

it("expands a burst into its children, each inspectable when a run is known", async () => {
  const events: { toolId: string }[] = [];
  const off = onFutureEvent("inspect-tool", detail => void events.push(detail));
  const children = [
    item({ id: "c1", kind: "shell", target: "npm test" }),
    item({ id: "c2", kind: "read", target: "/work/proj/src/b.ts" }),
  ];
  render(
    <AgentActivityLine
      item={item({ children, count: 2, kind: "shell", status: "completed" })}
      runId="R1"
      workspacePath="/work/proj"
    />,
  );
  expect(container.textContent).toBe("Ran 2 commands");

  await clickButton();
  expect(container.textContent).toContain("npm test");
  // A child file target is workspace-relative; the command is untouched.
  expect(container.textContent).toContain("src/b.ts");
  expect(container.textContent).not.toContain("/work/proj");

  // Each child row is its own inspect link.
  await clickButton(1);
  await clickButton(2);
  expect(events).toEqual([
    { runId: "R1", toolId: "c1" },
    { runId: "R1", toolId: "c2" },
  ]);
  off();
});

it("renders a burst's children as selectable text when there is no run", async () => {
  render(
    <AgentActivityLine
      item={item({ children: [item({ id: "c1", target: "ls -la" })], count: 1, kind: "shell" })}
      runId={null}
    />,
  );
  await clickButton();
  expect(buttons()).toHaveLength(1);
  expect(container.textContent).toContain("ls -la");
});

it("leaves a child with no target as an empty row rather than dropping it", async () => {
  // boundary: a persisted call can lose its target; the row must still exist.
  render(
    <AgentActivityLine
      item={item({ children: [item({ id: "c1", target: null as unknown as string })], count: 1, kind: "read" })}
      runId="R1"
    />,
  );
  await clickButton();
  expect(buttons()).toHaveLength(2);
  expect(buttons()[1]!.textContent).toBe("");
});

it("keeps each line's expand state to itself", async () => {
  // The pure dispatcher exists so a leaf and a group can each own `open`
  // without a rules-of-hooks violation.
  render(
    <AgentActivityList items={[
      item({ children: [item({ id: "g1c" })], count: 2, id: "g1", kind: "shell" }),
      item({ id: "s1", kind: "read" }),
    ]}
    />,
  );

  await clickButton(0);
  const lines = [...list()!.children] as HTMLElement[];
  expect(lines[0]!.textContent).toContain("Ran 2 commands");
  expect(lines[1]!.textContent).toBe("Read a file");
  expect(buttons().filter(button => button.getAttribute("aria-expanded") === "true")).toHaveLength(1);
});
