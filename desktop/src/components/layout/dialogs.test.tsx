// @vitest-environment jsdom
import type { ComponentProps, ReactElement } from "react";
import type { StoredThread, ThreadCleanupSummary } from "../../integrations/storage/threadStore";
import type { BatchDeleteDialogState, DeleteDialogState, RenameDialogState } from "./AppShellDialogs";
import type { WorkspaceDeleteDialogState, WorkspaceRenameDialogState } from "./hooks/useWorkspaceDialogs";
import { act, createElement, useState } from "react";
import { createRoot } from "react-dom/client";
import { describe, expect, it, vi } from "vitest";
import { AppShellDialogs } from "./AppShellDialogs";
import { ConfirmDeleteDialog, RenameDialog } from "./EntityDialogs";
import { LeftPanelTitlebarToggle } from "./LeftPanelTitlebarToggle";
import { WorkspaceDialogs } from "./WorkspaceDialogs";

const fullscreen = vi.hoisted(() => ({ value: false }));
const platform = vi.hoisted(() => ({ mac: false }));
vi.mock("../../lib/useIsFullscreen", () => ({ useIsFullscreen: () => fullscreen.value }));
vi.mock("../../lib/platform", () => ({
  get isMacOS() {
    return platform.mac;
  },
  isWindows: false,
  isLinux: false,
}));

function cleanupSummary(artifactCount: number): ThreadCleanupSummary {
  return {
    artifactCount,
    cleanupStatus: "active",
    threadId: "t1",
    workspaceFileCount: 0,
    workspaceId: "w-t1",
    workspaceKind: "user",
    workspacePath: "/tmp/t1",
  };
}

function thread(id: string, overrides: Partial<StoredThread> = {}): StoredThread {
  return {
    id,
    agentSessionId: id,
    title: `Title ${id}`,
    mode: "chat",
    workspaceId: `w-${id}`,
    status: "active",
    pinned: false,
    readonly: false,
    createdAt: 0,
    updatedAt: 0,
    ...overrides,
  };
}

function mount(node: ReactElement) {
  const container = document.createElement("div");
  document.body.appendChild(container);
  const root = createRoot(container);
  act(() => root.render(node));
  return {
    container,
    unmount: () => {
      act(() => root.unmount());
      container.remove();
    },
  };
}

/** Set an input's value the way a browser would, bypassing React's tracker. */
function typeInto(input: HTMLInputElement, value: string) {
  Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, value);
  input.dispatchEvent(new Event("input", { bubbles: true }));
}

const buttons = (container: HTMLElement) => [...container.querySelectorAll("button")];
function byText(container: HTMLElement, text: string) {
  return buttons(container).find(button => button.textContent === text);
}

describe("rename dialog", () => {
  function Harness(props: Partial<ComponentProps<typeof RenameDialog>>) {
    const { value: initialValue, ...rest } = props;
    const [value, setValue] = useState(initialValue ?? "");
    return createElement(RenameDialog, {
      description: "desc",
      label: "Name",
      onClose: () => {},
      onConfirm: () => {},
      open: true,
      submitting: false,
      title: "Rename Chat",
      error: null,
      ...rest,
      value,
      onChange: (next: string) => {
        rest.onChange?.(next);
        setValue(next);
      },
    });
  }

  it("submits on Enter and reports every keystroke", () => {
    const onChange = vi.fn();
    const onConfirm = vi.fn();
    const view = mount(createElement(Harness, { onChange, onConfirm }));
    const input = view.container.querySelector("input")!;
    typeInto(input, "A brand new name");
    expect(onChange).toHaveBeenCalledWith("A brand new name");
    expect(input.value).toBe("A brand new name");

    act(() => {
      input.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
    });
    expect(onConfirm).toHaveBeenCalledTimes(1);
    view.unmount();
  });

  it("ignores non-Enter keys", () => {
    const onConfirm = vi.fn();
    const view = mount(createElement(Harness, { onConfirm }));
    act(() => {
      view.container.querySelector("input")!.dispatchEvent(new KeyboardEvent("keydown", { key: "a", bubbles: true }));
    });
    expect(onConfirm).not.toHaveBeenCalled();
    view.unmount();
  });

  it("shows the error, a busy save label and disables the fields while submitting", () => {
    const onConfirm = vi.fn();
    const view = mount(createElement(Harness, { error: "Name already used", submitting: true, value: "Old" }));
    expect(view.container.querySelector("[role=\"alert\"]")!.textContent).toBe("Name already used");
    const busySave = byText(view.container, "Saving...")!;
    expect(view.container.querySelector("input")!.disabled).toBe(true);
    expect(busySave.disabled).toBe(true);
    act(() => busySave.click());
    expect(onConfirm).not.toHaveBeenCalled();
    view.unmount();
  });

  it("offers Auto-generate only when a generator is wired, and disables it while generating", () => {
    const without = mount(createElement(Harness, {}));
    expect(byText(without.container, "Auto-generate")).toBeUndefined();
    without.unmount();

    const onGenerate = vi.fn();
    const generating = mount(createElement(Harness, { onGenerate, generating: true }));
    expect(byText(generating.container, "Generating…")!.disabled).toBe(true);
    generating.unmount();

    const idle = mount(createElement(Harness, { onGenerate }));
    act(() => byText(idle.container, "Auto-generate")!.click());
    expect(onGenerate).toHaveBeenCalledTimes(1);
    idle.unmount();
  });

  it("closes from the Cancel button", () => {
    const onClose = vi.fn();
    const view = mount(createElement(Harness, { onClose }));
    act(() => byText(view.container, "Cancel")!.click());
    expect(onClose).toHaveBeenCalledTimes(1);
    view.unmount();
  });
});

describe("confirm delete dialog", () => {
  it("renders the body, the error and the busy label", () => {
    const view = mount(createElement(ConfirmDeleteDialog, {
      children: createElement("p", null, "body text"),
      error: "locked",
      onClose: () => {},
      onConfirm: () => {},
      open: true,
      submitting: true,
      title: "Delete Chat",
    }));
    expect(view.container.textContent).toContain("body text");
    expect(view.container.textContent).toContain("locked");
    const busyDelete = byText(view.container, "Deleting...")!;
    expect(busyDelete.disabled).toBe(true);
    view.unmount();
  });
});

describe("app shell dialogs", () => {
  interface HarnessState {
    batch?: BatchDeleteDialogState | null;
    del?: DeleteDialogState | null;
    rename?: RenameDialogState | null;
  }

  function Harness({ initial, handlers }: { initial: HarnessState; handlers: Record<string, () => void> }) {
    const [renameDialog, setRenameDialog] = useState(initial.rename ?? null);
    const [deleteDialog, setDeleteDialog] = useState(initial.del ?? null);
    const [batchDeleteDialog, setBatchDeleteDialog] = useState(initial.batch ?? null);
    return createElement(AppShellDialogs, {
      batchDeleteDialog,
      deleteDialog,
      renameDialog,
      setBatchDeleteDialog,
      setDeleteDialog,
      setRenameDialog,
      onConfirmBatchDeleteThread: handlers.batch ?? (() => {}),
      onConfirmDeleteThread: handlers.del ?? (() => {}),
      onConfirmRenameThread: handlers.rename ?? (() => {}),
      onGenerateTitle: handlers.generate ?? (() => {}),
    });
  }

  const renameState = (overrides: Partial<RenameDialogState> = {}): RenameDialogState =>
    ({ error: null, submitting: false, thread: thread("t1"), value: "Old", ...overrides });

  it("clears the error as the name is edited and closes on cancel", () => {
    const view = mount(createElement(Harness, { initial: { rename: renameState({ error: "dup" }) }, handlers: {} }));
    const input = view.container.querySelector("input")!;
    expect(input.value).toBe("Old");
    expect(view.container.querySelector("[role=\"alert\"]")!.textContent).toBe("dup");

    act(() => typeInto(input, "New name"));
    // The controlled input round-tripped through the shared setter: the new
    // value re-rendered and the previous error was cleared in the same update.
    expect(input.value).toBe("New name");
    expect(view.container.querySelector("[role=\"alert\"]")).toBeNull();

    act(() => byText(view.container, "Cancel")!.click());
    expect(view.container.querySelector("[role=\"dialog\"]")).toBeNull();
    view.unmount();
  });

  it("drops an input edit that lands in the same batch as the dismissal", () => {
    // The onChange updater is `current => current ? { …current, value } : current`.
    // When a change event and the dismissal are processed in the same batch, the
    // dismissal can be applied first, so the edit must be dropped rather than
    // resurrect the closed dialog (or throw on a null `current`).
    const view = mount(createElement(Harness, { initial: { rename: renameState() }, handlers: {} }));
    const input = view.container.querySelector("input")!;

    act(() => {
      // Dismiss first, then the in-flight edit — both before React flushes, which
      // is exactly the ordering that leaves the updater holding a null state.
      byText(view.container, "Cancel")!.click();
      typeInto(input, "typed too late");
    });

    // The dialog stays closed: the late edit did not bring it back.
    expect(view.container.querySelector("[role=\"dialog\"]")).toBeNull();
    view.unmount();
  });

  it("wires the rename confirm and generate buttons", () => {
    const rename = vi.fn();
    const generate = vi.fn();
    const view = mount(createElement(Harness, { initial: { rename: renameState() }, handlers: { rename, generate } }));
    act(() => byText(view.container, "Auto-generate")!.click());
    expect(generate).toHaveBeenCalledTimes(1);
    act(() => byText(view.container, "Save")!.click());
    expect(rename).toHaveBeenCalledTimes(1);
    view.unmount();
  });

  it("describes a chat delete with its artifact cleanup count and confirms", () => {
    const del = vi.fn();
    const view = mount(createElement(Harness, {
      initial: {
        del: { cleanupSummary: cleanupSummary(3), error: null, loadingSummary: false, submitting: false, thread: thread("t1") },
      },
      handlers: { del },
    }));
    expect(view.container.textContent).toContain("This chat will be removed from the sidebar.");
    expect(view.container.textContent).toContain("Artifacts");
    expect(view.container.textContent).toContain("3");
    act(() => byText(view.container, "Delete")!.click());
    expect(del).toHaveBeenCalledTimes(1);

    act(() => byText(view.container, "Cancel")!.click());
    expect(view.container.querySelector("[role=\"dialog\"]")).toBeNull();
    view.unmount();
  });

  it("closes the chat delete dialog from Cancel without deleting", () => {
    const del = vi.fn();
    const view = mount(createElement(Harness, {
      initial: {
        del: { cleanupSummary: null, error: null, loadingSummary: false, submitting: false, thread: thread("t1") },
      },
      handlers: { del },
    }));
    act(() => byText(view.container, "Cancel")!.click());
    expect(view.container.querySelector("[role=\"dialog\"]")).toBeNull();
    expect(del).not.toHaveBeenCalled();
    view.unmount();
  });

  it("describes a workspace delete and skips the artifact block", () => {
    const view = mount(createElement(Harness, {
      initial: {
        del: { cleanupSummary: cleanupSummary(5), error: "nope", loadingSummary: false, submitting: false, thread: thread("t1", { mode: "workspace" }) },
      },
      handlers: {},
    }));
    expect(view.container.textContent).toContain("This removes only the chat. Workspace files will not be changed.");
    expect(view.container.textContent).not.toContain("Artifacts");
    expect(view.container.textContent).toContain("nope");
    view.unmount();
  });

  it("summarises a mixed batch delete, truncating the list and offering the file toggle", () => {
    const threads = [
      thread("a"),
      thread("b"),
      thread("c"),
      thread("d"),
      thread("e"),
      thread("f"),
      thread("g", { mode: "workspace" }),
      thread("h", { mode: "workspace" }),
    ];
    const batch = vi.fn();
    const view = mount(createElement(Harness, {
      initial: {
        batch: { chatThreadCount: 6, deleteFiles: false, error: null, submitting: false, threads, workspaceThreadCount: 2 },
      },
      handlers: { batch },
    }));
    expect(view.container.textContent).toContain("8 conversation(s) selected");
    expect(view.container.textContent).toContain("6 chat(s) and 2 workspace conversation(s) will be removed");
    expect(view.container.textContent).toContain("2 workspace conversation(s): files on disk will not be deleted.");
    // Only the first five titles are listed; the rest collapse into a count.
    expect(view.container.textContent).toContain("Title a");
    expect(view.container.textContent).not.toContain("Title f");
    expect(view.container.textContent).toContain("and 3 more");

    const checkbox = view.container.querySelector("input[type=\"checkbox\"]")! as HTMLInputElement;
    expect(checkbox.checked).toBe(false);
    act(() => checkbox.click());
    expect(checkbox.checked).toBe(true);

    act(() => byText(view.container, "Delete")!.click());
    expect(batch).toHaveBeenCalledTimes(1);
    view.unmount();
  });

  it("drops a file-toggle change that lands in the same batch as the dismissal", () => {
    // Same defensive updater as the rename value, on the batch dialog's
    // "also delete files" checkbox: a toggle processed after the dismissal must
    // not reopen the dialog.
    const view = mount(createElement(Harness, {
      initial: {
        batch: { chatThreadCount: 3, deleteFiles: false, error: null, submitting: false, threads: [thread("a")], workspaceThreadCount: 0 },
      },
      handlers: {},
    }));
    const checkbox = view.container.querySelector<HTMLInputElement>("input[type=\"checkbox\"]")!;

    act(() => {
      byText(view.container, "Cancel")!.click();
      checkbox.click();
    });

    expect(view.container.querySelector("[role=\"dialog\"]")).toBeNull();
    view.unmount();
  });

  it("lists every thread when the batch is small and hides the file toggle for workspace-only batches", () => {
    const view = mount(createElement(Harness, {
      initial: {
        batch: {
          chatThreadCount: 0,
          deleteFiles: false,
          error: null,
          submitting: false,
          threads: [thread("ws-1", { mode: "workspace" })],
          workspaceThreadCount: 1,
        },
      },
      handlers: {},
    }));
    expect(view.container.textContent).toContain("Title ws-1");
    expect(view.container.textContent).not.toContain("more");
    expect(view.container.querySelector("input[type=\"checkbox\"]")).toBeNull();
    // Pure-workspace batches use the workspace wording.
    expect(view.container.textContent).toContain("This removes only the chat.");
    view.unmount();
  });

  it("describes a chat-only batch, offers the file toggle and closes on cancel", () => {
    const view = mount(createElement(Harness, {
      initial: {
        batch: {
          chatThreadCount: 2,
          deleteFiles: false,
          error: null,
          submitting: false,
          threads: [thread("a"), thread("b")],
          workspaceThreadCount: 0,
        },
      },
      handlers: {},
    }));
    expect(view.container.textContent).toContain("This chat will be removed from the sidebar.");
    expect(view.container.textContent).not.toContain("workspace conversation(s)");

    act(() => byText(view.container, "Cancel")!.click());
    expect(view.container.querySelector("[role=\"dialog\"]")).toBeNull();
    view.unmount();
  });

  it("renders nothing when every dialog is closed", () => {
    const view = mount(createElement(Harness, { initial: {}, handlers: {} }));
    expect(view.container.querySelector("[role=\"dialog\"]")).toBeNull();
    expect(view.container.textContent).toBe("");
    view.unmount();
  });
});

describe("workspace dialogs", () => {
  const workspace = { cleanupStatus: "active" as const, createdAt: 0, id: "ws-1", kind: "user" as const, name: "Alpha", path: "/tmp/alpha", pinned: false, updatedAt: 0 };

  function Harness({ handlers, initial }: { handlers: Record<string, () => void>; initial?: { deleteOpen?: boolean } }) {
    const [renameDialog, setRenameDialog] = useState<WorkspaceRenameDialogState | null>(
      { error: "taken", submitting: false, value: "Alpha", workspace },
    );
    const [deleteDialog, setDeleteDialog] = useState<WorkspaceDeleteDialogState | null>(
      initial?.deleteOpen ? { error: null, submitting: false, workspace } : null,
    );
    return createElement(WorkspaceDialogs, {
      deleteDialog,
      renameDialog,
      setDeleteDialog,
      setRenameDialog,
      onConfirmDeleteWorkspace: handlers.del ?? (() => {}),
      onConfirmRenameWorkspace: handlers.rename ?? (() => {}),
    });
  }

  it("shows the workspace name in the delete confirmation and confirms it", () => {
    const del = vi.fn();
    const view = mount(createElement(Harness, { handlers: { del }, initial: { deleteOpen: true } }));
    expect(view.container.textContent).toContain("Delete workspace");
    expect(view.container.textContent).toContain("Alpha");
    act(() => byText(view.container, "Delete")!.click());
    expect(del).toHaveBeenCalledTimes(1);

    const deleteDialog = [...view.container.querySelectorAll("[role=\"dialog\"]")]
      .find(dialog => dialog.textContent!.includes("Delete workspace"))!;
    const cancel = [...deleteDialog.querySelectorAll<HTMLButtonElement>("button")]
      .find(button => button.textContent === "Cancel")!;
    act(() => cancel.click());
    expect([...view.container.querySelectorAll("[role=\"dialog\"]")]
      .some(dialog => dialog.textContent!.includes("Delete workspace"))).toBe(false);
    view.unmount();
  });

  it("drops a workspace name edit that lands in the same batch as the dismissal", () => {
    // The workspace rename dialog shares the rename dialog's null-safe updater.
    const view = mount(createElement(Harness, { handlers: {} }));
    const input = view.container.querySelector("input")!;

    act(() => {
      byText(view.container, "Cancel")!.click();
      typeInto(input, "typed too late");
    });

    expect([...view.container.querySelectorAll("[role=\"dialog\"]")]
      .some(dialog => dialog.textContent!.includes("Rename workspace"))).toBe(false);
    view.unmount();
  });

  it("clears the error as the name is edited and confirms a rename", () => {
    const rename = vi.fn();
    const view = mount(createElement(Harness, { handlers: { rename } }));
    expect(view.container.textContent).toContain("Rename workspace");
    expect(view.container.querySelector("[role=\"alert\"]")!.textContent).toBe("taken");
    act(() => typeInto(view.container.querySelector("input")!, "Beta"));
    expect(view.container.querySelector("[role=\"alert\"]")).toBeNull();
    act(() => byText(view.container, "Save")!.click());
    expect(rename).toHaveBeenCalledTimes(1);
    act(() => byText(view.container, "Cancel")!.click());
    expect(view.container.querySelector("[role=\"dialog\"]")).toBeNull();
    view.unmount();
  });
});

describe("left panel titlebar toggle", () => {
  it("renders nothing while the rail is expanded", () => {
    const view = mount(createElement(LeftPanelTitlebarToggle, { expanded: true, onToggle: () => {} }));
    expect(view.container.textContent).toBe("");
    view.unmount();
  });

  it("exposes a labelled show-sidebar button that toggles", () => {
    const onToggle = vi.fn();
    const view = mount(createElement(LeftPanelTitlebarToggle, { expanded: false, onToggle }));
    const button = view.container.querySelector("button")!;
    expect(button.getAttribute("aria-label")).toBe("Show sidebar");
    expect(button.getAttribute("title")).toBe("Show sidebar");
    act(() => button.click());
    expect(onToggle).toHaveBeenCalledTimes(1);
    view.unmount();
  });

  it("swallows the drag mousedown before it reaches an ancestor titlebar handler", () => {
    const onParentDown = vi.fn();
    function Wrapper() {
      return createElement(
        "div",
        { onMouseDown: onParentDown },
        createElement(LeftPanelTitlebarToggle, { expanded: false, onToggle: () => {} }),
      );
    }
    const view = mount(createElement(Wrapper));
    act(() => {
      view.container.querySelector("button")!.dispatchEvent(new MouseEvent("mousedown", { bubbles: true }));
    });
    expect(onParentDown).not.toHaveBeenCalled();
    view.unmount();
  });

  it("reserves the macOS traffic-light inset only outside fullscreen", () => {
    platform.mac = false;
    const plain = mount(createElement(LeftPanelTitlebarToggle, { expanded: false, onToggle: () => {} }));
    expect(plain.container.querySelector("div")!.classList.contains("pl-16")).toBe(false);
    plain.unmount();

    platform.mac = true;
    fullscreen.value = false;
    const mac = mount(createElement(LeftPanelTitlebarToggle, { expanded: false, onToggle: () => {} }));
    expect(mac.container.querySelector("div")!.classList.contains("pl-16")).toBe(true);
    mac.unmount();

    fullscreen.value = true;
    const macFull = mount(createElement(LeftPanelTitlebarToggle, { expanded: false, onToggle: () => {} }));
    expect(macFull.container.querySelector("div")!.classList.contains("pl-16")).toBe(false);
    macFull.unmount();
    platform.mac = false;
    fullscreen.value = false;
  });
});
