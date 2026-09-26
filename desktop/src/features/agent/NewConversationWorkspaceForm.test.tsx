// @vitest-environment jsdom
import type { FormEvent } from "react";
import type { Root } from "react-dom/client";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { WorkspaceModal } from "./NewConversationWorkspaceForm";

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

function render(overrides: Partial<Parameters<typeof WorkspaceModal>[0]> = {}) {
  const handlers = {
    onCancel: vi.fn(),
    onDisplayNameChange: vi.fn(),
    onPickFolder: vi.fn(),
    onSubmit: vi.fn(),
  };
  act(() => root.render(
    <WorkspaceModal
      creating={false}
      displayName=""
      error={null}
      notice={null}
      path=""
      {...handlers}
      {...overrides}
    />,
  ));
  return handlers;
}

function byText(text: string) {
  return [...container.querySelectorAll("button")]
    .find(button => button.textContent?.trim() === text)!;
}

function chooseButton() {
  return container.querySelector<HTMLButtonElement>("button[aria-label='Choose workspace']")!;
}

function nameInput() {
  return container.querySelector<HTMLInputElement>("input")!;
}

it("prompts for a directory while no path is chosen and previews the real one afterwards", () => {
  render();
  expect(container.textContent).toContain("Open Existing Workspace");
  expect(container.textContent).toContain("Select existing workspace");
  expect(container.textContent).toContain("Choose an existing workspace directory.");
  expect(container.textContent).not.toContain("Workspace path:");

  act(() => root.unmount());
  root = createRoot(container);
  render({ path: "/home/me/proj" });
  expect(container.textContent).toContain("/home/me/proj");
  expect(container.textContent).toContain("Workspace path: /home/me/proj");
  expect(container.textContent).not.toContain("Select existing workspace");
});

it("reports every interaction to its owner", () => {
  const handlers = render({ displayName: "proj" });
  expect(nameInput().value).toBe("proj");

  act(() => {
    chooseButton().click();
    byText("Cancel").click();
  });
  expect(handlers.onPickFolder).toHaveBeenCalledTimes(1);
  expect(handlers.onCancel).toHaveBeenCalledTimes(1);

  // The name field is controlled: the parent owns the value. React's value
  // tracker suppresses an `input` event whose value it already knows, so write
  // through the native setter the way a real keystroke does.
  act(() => {
    const input = nameInput();
    const setValue = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!;
    setValue.call(input, "renamed");
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
  expect(handlers.onDisplayNameChange).toHaveBeenCalledWith("renamed");
});

it("submits through the form rather than the button only", () => {
  const handlers = render({ path: "/tmp/x" });
  act(() => {
    container.querySelector("form")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
  });
  expect(handlers.onSubmit).toHaveBeenCalledTimes(1);
  expect((handlers.onSubmit.mock.calls[0]![0] as FormEvent).type).toBe("submit");
});

it("shows the error instead of the notice, never both", () => {
  // boundary: an error and a stale notice can coexist in state; the error wins
  // because the notice would contradict it.
  render({ error: "Choose a workspace directory.", notice: "\"proj\" already exists." });
  expect(container.textContent).toContain("Choose a workspace directory.");
  expect(container.textContent).not.toContain("already exists");

  act(() => root.unmount());
  root = createRoot(container);
  render({ notice: "\"proj\" already exists." });
  expect(container.textContent).toContain("already exists");
});

it("blocks a second submit while the workspace is being created", () => {
  render({ creating: true, path: "/tmp/x" });
  const submit = container.querySelector<HTMLButtonElement>("button[type=submit]")!;
  expect(submit.disabled).toBe(true);
  expect(submit.textContent).toBe("Open");

  act(() => root.unmount());
  root = createRoot(container);
  render({ path: "/tmp/x" });
  expect(container.querySelector<HTMLButtonElement>("button[type=submit]")!.disabled).toBe(false);
});
