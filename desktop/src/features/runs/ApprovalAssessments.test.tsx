import type { StoredApprovalAssessment } from "../../integrations/storage/runs";
// @vitest-environment jsdom
import type { StoredRun } from "../../integrations/storage/threadStore";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { listApprovalAssessments } from "../../integrations/storage/runs";
import { ApprovalAssessments } from "./ApprovalAssessments";

vi.mock("../../integrations/storage/runs", () => ({ listApprovalAssessments: vi.fn(async () => []) }));
(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;
it("shows terminal decisions and their sanitized action only for the inspected run/tool", async () => {
  const entry: StoredApprovalAssessment = { id: "review", runId: "r", toolCallId: "tool", status: "rejected", createdAt: 1, payload: { effective: { risk: "high", authorization: "low" }, reported: { risk: "low", authorization: "high", reason_code: "authorization_scope_mismatch" }, confidence: { risk: 0.96 }, action: { command: "sanitized command" }, action_digest: "sha256:fixture", model: "jev-fixture", prompt_version: 1, reason_catalog_version: 1, policy_version: 1, duration_ms: 10 } };
  vi.mocked(listApprovalAssessments).mockResolvedValue([entry]);
  const container = document.createElement("div");
  const root = createRoot(container);
  const render = async (runId: string, toolCallId?: string) => {
    await act(async () => root.render(<ApprovalAssessments run={{ id: runId, status: "completed" } as StoredRun} toolCallId={toolCallId} />));
  };
  try {
    await render("r", "tool");
    expect(container.textContent).toContain("Rejected");
    expect(container.textContent).toContain("High");
    expect(container.textContent).toContain("Outside authorized scope");
    expect(container.textContent).toContain("sanitized command");
    expect(container.textContent).toContain("96%");
    await render("r", "other");
    expect(container.textContent).toBe("");
    await render("other");
    expect(container.textContent).toBe("");
  }
  finally { act(() => root.unmount()); }
});
