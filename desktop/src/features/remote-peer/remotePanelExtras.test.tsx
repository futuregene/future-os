// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { beforeEach, expect, it, vi } from "vitest";
import { RemoteFilesPanel } from "./RemoteFilesPanel";

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

const list = vi.fn();
vi.mock("./remotePeerClient", () => ({
  listRemoteSessionFiles: (...args: unknown[]) => list(...args),
  downloadRemoteFile: vi.fn(),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ save: vi.fn(async () => null) }));

beforeEach(() => {
  list.mockReset().mockResolvedValue({ entries: [], path: "/home/studio", rootPath: "/home/studio" });
});

/** A refresh that fails says so, rather than leaving the old listing looking current. */
it("surfaces a refresh failure", async () => {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  await act(async () => {
    root.render(
      <RemoteFilesPanel desktopId="desktop_a" expanded onToggleExpanded={vi.fn()} peer={undefined} sessionId="sess_1" />,
    );
  });

  list.mockRejectedValue(new Error("the host went away"));
  await act(async () => container.querySelector<HTMLButtonElement>("button[aria-label='Re-read this folder']")!.click());
  expect(container.textContent).toContain("the host went away");

  await act(async () => root.unmount());
  container.remove();
});
