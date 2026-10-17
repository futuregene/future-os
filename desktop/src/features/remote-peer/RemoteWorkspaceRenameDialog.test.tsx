// @vitest-environment jsdom
import type { RemoteWorkspaceTarget } from "./mergeConversations";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { beforeEach, expect, it, vi } from "vitest";
import { RemoteWorkspaceRenameDialog } from "./RemoteWorkspaceRenameDialog";

/**
 * Renaming a workspace on another machine.
 *
 * Two arms of `submit` are reachable from this dialog and neither is guarded by
 * the button that triggers it, which is worth pinning: the shared `RenameDialog`
 * shell closes Save on `submitting` alone, so an emptied name still reaches the
 * handler, and the host's own refusal has to stay in the dialog (closing it would
 * hide the only explanation the user gets).
 */

const rename = vi.fn<(desktopId: string, workspaceId: string, name: string) => Promise<void>>();
vi.mock("./remotePeerClient", () => ({
  renameRemoteWorkspace: (...args: Parameters<typeof rename>) => rename(...args),
}));

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

function target(overrides: Partial<RemoteWorkspaceTarget> = {}): RemoteWorkspaceTarget {
  return { desktopId: "desktop_a", name: "Old name", workspaceId: "ws_1", ...overrides };
}

beforeEach(() => {
  rename.mockReset();
  rename.mockResolvedValue(undefined);
});

async function mount(row = target()) {
  const onClose = vi.fn();
  const onRenamed = vi.fn();
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  await act(async () => {
    root.render(<RemoteWorkspaceRenameDialog onClose={onClose} onRenamed={onRenamed} target={row} />);
  });
  const input = container.querySelector("input")!;
  return {
    container,
    input,
    onClose,
    onRenamed,
    save: () => [...container.querySelectorAll("button")].find(node => node.textContent === "Save")!,
    setValue: async (value: string) => {
      await act(async () => {
        const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!;
        setter.call(input, value);
        input.dispatchEvent(new Event("input", { bubbles: true }));
      });
    },
    clickSave: async () => {
      await act(async () => {
        [...container.querySelectorAll("button")].find(node => node.textContent === "Save")!.click();
        for (let i = 0; i < 4; i += 1) await Promise.resolve();
      });
    },
    text: () => container.textContent ?? "",
    unmount: async () => {
      await act(async () => root.unmount());
      container.remove();
    },
  };
}

it("sends the trimmed new name to the host that owns the workspace, then closes", async () => {
  const dialog = await mount();
  await dialog.setValue("  Renamed workspace  ");
  await dialog.clickSave();

  expect(rename).toHaveBeenCalledWith("desktop_a", "ws_1", "Renamed workspace");
  expect(dialog.onRenamed).toHaveBeenCalledTimes(1);
  expect(dialog.onClose).toHaveBeenCalledTimes(1);
  await dialog.unmount();
});

/**
 * An emptied name is refused locally: no round trip, and nothing to close.
 *
 * The shared shell's Save is closed on `submitting` and **not** on an empty
 * value, so this is a real click rather than a guard duplicated by the button
 * (the comment above `submit` claimed the button covers it too — it does not).
 * Sending it would reach a host that refuses an empty name anyway; the point of
 * the local check is that the user sees nothing happen instead of an error.
 */
it("sends nothing when the name is emptied", async () => {
  const dialog = await mount();
  await dialog.setValue("   ");
  expect(dialog.save().disabled).toBe(false);

  await dialog.clickSave();

  expect(rename).not.toHaveBeenCalled();
  expect(dialog.onRenamed).not.toHaveBeenCalled();
  expect(dialog.onClose).not.toHaveBeenCalled();
  expect(dialog.text()).not.toContain("alert");
  await dialog.unmount();
});

/** The host's refusal stays in the dialog: closing would hide its reason. */
it("keeps the dialog open with the host's refusal", async () => {
  rename.mockRejectedValue(new Error("workspace ws_1 does not exist"));
  const dialog = await mount();
  await dialog.setValue("Renamed workspace");
  await dialog.clickSave();

  expect(dialog.text()).toContain("workspace ws_1 does not exist");
  expect(dialog.onRenamed).not.toHaveBeenCalled();
  expect(dialog.onClose).not.toHaveBeenCalled();

  // And the next attempt is sendable again — the busy flag was cleared.
  expect(dialog.save().disabled).toBe(false);
  expect(dialog.input.value).toBe("Renamed workspace");
  await dialog.unmount();
});
