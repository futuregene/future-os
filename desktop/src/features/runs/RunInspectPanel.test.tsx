// @vitest-environment jsdom
import type { StoredRun, StoredToolCall } from "../../integrations/storage/types";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { listToolOutputs } from "../../integrations/storage/threadStore";
import { RunInspectPanel } from "./RunInspectPanel";

vi.mock("../../integrations/storage/threadStore", async importOriginal => ({
  ...await importOriginal<typeof import("../../integrations/storage/threadStore")>(),
  listToolOutputs: vi.fn(async () => []),
}));
(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

it("does not refetch settled outputs for freshly allocated equivalent tool props", async () => {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const run = { id: "r1", status: "completed", createdAt: 1 } as StoredRun;
  const tool: StoredToolCall = { id: "t1", runId: "r1", name: "shell", kind: "shell", status: "completed", createdAt: 1, endedAt: 2 };
  const render = async () => {
    await act(async () => root.render(<RunInspectPanel compact run={run} tools={[{ ...tool }]} onBack={() => {}} />));
  };
  try {
    await render();
    await render();
    await render();
    expect(listToolOutputs).toHaveBeenCalledTimes(1);
    tool.endedAt = 3;
    await render();
    expect(listToolOutputs).toHaveBeenCalledTimes(2);
  }
  finally {
    act(() => root.unmount());
    container.remove();
  }
});
