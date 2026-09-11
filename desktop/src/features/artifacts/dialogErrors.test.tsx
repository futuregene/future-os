// @vitest-environment jsdom
import type { StoredArtifact } from "../../integrations/storage/types";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { ArtifactDetailPanel } from "./ArtifactDetailPanel";
import { ArtifactsPanel } from "./ArtifactsPanel";

vi.mock("@tauri-apps/plugin-dialog", () => ({
  open: async () => { throw new Error("open dialog unavailable"); },
  save: async () => { throw new Error("save dialog unavailable"); },
}));
(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

it.each([false, true])("shows file dialog rejections (export=%s)", async (exporting) => {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  try {
    const artifact: StoredArtifact = { id: "a1", workspaceId: "w1", title: "artifact", createdAt: 1, updatedAt: 1, artifactType: "text", content: "sample" };
    await act(async () => root.render(exporting
      ? <ArtifactDetailPanel artifact={artifact} onBack={() => {}} onChanged={() => {}} />
      : <ArtifactsPanel artifacts={[]} threadId="t1" workspacePath={null} onChanged={() => {}} onSelectArtifact={() => {}} />));
    const button = [...container.querySelectorAll("button")].find(item => item.textContent === (exporting ? "Export" : "Upload"))!;
    expect(button).toBeTruthy();
    await act(async () => button.click());
    expect(container.textContent).toContain(exporting ? "save dialog unavailable" : "open dialog unavailable");
    expect(button.disabled).toBe(false);
  }
  finally {
    act(() => root.unmount());
    container.remove();
  }
});
