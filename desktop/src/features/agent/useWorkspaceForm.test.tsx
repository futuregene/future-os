// @vitest-environment jsdom
import type { FormEvent } from "react";
import type { StoredWorkspace } from "../../integrations/storage/threadStore";
import { act } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { renderHook } from "../../test/renderHook";
import { useWorkspaceForm } from "./useWorkspaceForm";

const dialog = vi.hoisted(() => ({ open: vi.fn() }));

vi.mock("@tauri-apps/plugin-dialog", () => ({
  open: dialog.open,
}));

afterEach(() => {
  dialog.open.mockReset();
});

interface AddInput { name?: string | null; path: string; createDirectory: boolean }

function workspace(id: string, path: string, name = id): StoredWorkspace {
  return { id, path, name, createdAt: 0, updatedAt: 0 } as StoredWorkspace;
}

/** A `FormEvent` stand-in: the hook only calls `preventDefault()`. */
function formEvent(): FormEvent {
  return { preventDefault: vi.fn() } as unknown as FormEvent;
}

function setup(overrides: {
  initialWorkspaceForm?: "open" | null;
  workspaces?: StoredWorkspace[];
  onAddWorkspace?: (input: AddInput) => Promise<StoredWorkspace | null>;
} = {}) {
  const calls = {
    add: [] as AddInput[],
    selected: [] as string[],
    modes: [] as string[],
    closed: 0,
  };
  const onAddWorkspace = overrides.onAddWorkspace
    ?? (async (input: AddInput) => {
      calls.add.push(input);
      return workspace("w-new", input.path, input.name ?? "dir");
    });
  const harness = renderHook(() => useWorkspaceForm({
    initialWorkspaceForm: overrides.initialWorkspaceForm,
    workspaces: overrides.workspaces ?? [],
    onAddWorkspace,
    onSelectWorkspace: id => void calls.selected.push(id),
    onModeChange: mode => void calls.modes.push(mode),
    onCloseMenu: () => void (calls.closed += 1),
  }));
  const name = (value: string) => act(() => harness.current.setDisplayName(value));
  /** The only way the path is ever filled: the native folder picker. */
  const pick = async (path: string | null) => {
    dialog.open.mockResolvedValue(path);
    await act(async () => harness.current.pickFolder());
  };
  const submit = async () => {
    const event = formEvent();
    await act(async () => {
      await harness.current.submit(event);
    });
    return event;
  };
  return { harness, calls, name, pick, submit };
}

describe("useWorkspaceForm", () => {
  it("starts closed, or already open when the caller asked for it", () => {
    const closed = setup();
    expect(closed.harness.current.mode).toBeNull();
    closed.harness.unmount();

    const open = setup({ initialWorkspaceForm: "open" });
    expect(open.harness.current.mode).toBe("open");
    expect(open.harness.current.creating).toBe(false);
    expect(open.harness.current.error).toBeNull();
    expect(open.harness.current.notice).toBeNull();
    open.harness.unmount();
  });

  it("refuses a submit with no directory and never calls the backend", async () => {
    const blank = setup();
    const blankEvent = await blank.submit();
    expect(blankEvent.preventDefault).toHaveBeenCalled();
    expect(blank.harness.current.error).toBe("Choose a workspace directory.");
    expect(blank.harness.current.creating).toBe(false);
    expect(blank.calls.add).toHaveLength(0);
    blank.harness.unmount();

    // boundary: whitespace-only counts as empty, same as untouched.
    const spaces = setup();
    await spaces.pick("   ");
    await spaces.submit();
    expect(spaces.harness.current.error).toBe("Choose a workspace directory.");
    expect(spaces.calls.add).toHaveLength(0);
    spaces.harness.unmount();
  });

  it("trims the fields, selects the created workspace and closes the menu", async () => {
    const { harness, calls, name, pick, submit } = setup();
    act(() => harness.current.begin());
    expect(harness.current.mode).toBe("open");
    expect(calls.closed).toBe(1);

    name("  My Project  ");
    await pick("  C:\\work\\proj  ");
    await submit();

    expect(calls.add).toEqual([{ name: "My Project", path: "C:\\work\\proj", createDirectory: false }]);
    expect(calls.selected).toEqual(["w-new"]);
    expect(calls.modes).toEqual(["workspace"]);
    expect(harness.current.mode).toBeNull();
    expect(harness.current.path).toBe("");
    expect(harness.current.displayName).toBe("");
    expect(calls.closed).toBe(2);
    harness.unmount();
  });

  it("sends a null name when the user left the name blank", async () => {
    // The picker pre-fills a name; clearing it must send `null` (the backend
    // then names the workspace itself) rather than an empty string.
    const { harness, calls, name, pick, submit } = setup();
    await pick("/tmp/only-path");
    expect(harness.current.displayName).toBe("only-path");
    name("   ");
    await submit();

    expect(calls.add).toEqual([{ name: null, path: "/tmp/only-path", createDirectory: false }]);
    harness.unmount();
  });

  it("closes the form but selects nothing when the backend declines the workspace", async () => {
    // error-path: the backend deduped the path away and returned nothing.
    const { harness, calls, pick, submit } = setup({ onAddWorkspace: async () => null });
    await pick("/tmp/dup");
    await submit();

    expect(calls.selected).toEqual([]);
    expect(calls.modes).toEqual([]);
    expect(harness.current.mode).toBeNull();
    expect(harness.current.error).toBeNull();
    harness.unmount();
  });

  it("surfaces a backend failure and leaves the form usable", async () => {
    // error-path: the create call rejects; the dialog stays open with the
    // message and the draft is preserved so the user can retry.
    const { harness, pick, submit } = setup({
      onAddWorkspace: async () => { throw new Error("path is not a directory"); },
    });
    act(() => harness.current.begin());
    await pick("/tmp/file.txt");
    await submit();

    expect(harness.current.error).toBe("path is not a directory");
    expect(harness.current.mode).toBe("open");
    expect(harness.current.path).toBe("/tmp/file.txt");
    expect(harness.current.creating).toBe(false);
    harness.unmount();
  });

  it("falls back to the most recent workspace on cancel, or to chat when there is none", () => {
    const list = [workspace("recent", "/work/recent"), workspace("older", "/work/older")];
    const withWorkspaces = setup({ workspaces: list });
    act(() => withWorkspaces.harness.current.begin());
    act(() => withWorkspaces.harness.current.cancel());
    expect(withWorkspaces.calls.selected).toEqual(["recent"]);
    expect(withWorkspaces.calls.modes).toEqual(["workspace"]);
    expect(withWorkspaces.harness.current.mode).toBeNull();
    withWorkspaces.harness.unmount();

    // boundary: an empty list has nothing to land on.
    const empty = setup({ workspaces: [] });
    act(() => empty.harness.current.begin());
    act(() => empty.harness.current.cancel());
    expect(empty.calls.selected).toEqual([]);
    expect(empty.calls.modes).toEqual(["chat"]);
    empty.harness.unmount();
  });

  it("clears a previous error, notice and draft when the form is reopened", async () => {
    const { harness, pick, submit } = setup();
    await submit(); // no path: sets the error state
    expect(harness.current.error).not.toBeNull();
    await pick("/tmp/x");
    expect(harness.current.path).toBe("/tmp/x");
    act(() => harness.current.begin());
    expect(harness.current.error).toBeNull();
    expect(harness.current.notice).toBeNull();
    expect(harness.current.path).toBe("");
    expect(harness.current.displayName).toBe("");
    harness.unmount();
  });

  it("fills the path and derives a name from the picked folder", async () => {
    const { harness, pick } = setup();
    await pick("/home/me/sources/rocket");

    expect(dialog.open).toHaveBeenCalledWith({
      directory: true,
      multiple: false,
      title: "Open workspace",
    });
    expect(harness.current.path).toBe("/home/me/sources/rocket");
    expect(harness.current.displayName).toBe("rocket");
    expect(harness.current.notice).toBeNull();
    harness.unmount();
  });

  it("flags an already-registered folder ignoring case and a trailing separator", async () => {
    // platform-cfg: macOS/Windows paths are case-insensitive and a picked path
    // may carry a trailing separator; both spellings are the same directory.
    const existing = workspace("existing", "C:/Work/App/", "App");
    const { harness, name, pick } = setup({ workspaces: [existing] });
    name("My App");
    await pick("c:/work/app");

    expect(harness.current.notice).toBe("\"App\" already exists; the existing workspace will be opened.");
    // The user's typed name wins over the folder-derived one.
    expect(harness.current.displayName).toBe("My App");
    harness.unmount();
  });

  it("does not flag a genuinely different folder", async () => {
    const { harness, pick } = setup({ workspaces: [workspace("other", "/work/other")] });
    await pick("/work/different");

    expect(harness.current.notice).toBeNull();
    harness.unmount();
  });

  it("falls back to a literal name when the picked folder has no final segment", async () => {
    // boundary: a separator-only path has no basename to derive a name from.
    const { harness, pick } = setup();
    await pick("/");
    expect(harness.current.path).toBe("/");
    expect(harness.current.displayName).toBe("Workspace");
    harness.unmount();
  });

  it("leaves the form untouched when the folder picker is dismissed", async () => {
    // boundary: the dialog resolves `null` on cancel.
    const { harness, pick } = setup();
    act(() => harness.current.begin());
    await pick(null);

    expect(harness.current.path).toBe("");
    expect(harness.current.displayName).toBe("");
    expect(harness.current.notice).toBeNull();
    expect(harness.current.mode).toBe("open");
    harness.unmount();
  });
});
