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
it("shows user-facing decisions only for the inspected run/tool", async () => {
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
    for (const internal of ["sanitized command", "96%", "jev-fixture", "sha256:fixture", "Audit", "Confidence"])
      expect(container.textContent).not.toContain(internal);
    await render("r", "other");
    expect(container.textContent).toBe("");
    await render("other");
    expect(container.textContent).toBe("");
  }
  finally { act(() => root.unmount()); }
});

it("explains unexecuted actions without exposing probabilities or diagnostics", async () => {
  const entry: StoredApprovalAssessment = { id: "review", runId: "r", toolCallId: "tool", status: "review_uncertain", createdAt: 1, payload: { effective: { risk: "low", authorization: "high" }, reported: { risk: "low", authorization: "high", reason_code: "routine_bounded_action" }, confidence: { risk: 1, authorization: 0.32, authorization_support: 0.99, reason_code: 0.97 }, action: { path: "/example/Desktop/test.txt" }, action_digest: "sha256:fixture", error_code: "provider_invalid_response", model: "jev-fixture", prompt_version: 3, reason_catalog_version: 2, policy_version: 2, duration_ms: 10 } };
  const cases: [string, string][] = [
    ["review_uncertain", "Unable to confirm this action can proceed. It was not run."],
    ["review_error", "Review is temporarily unavailable. This action was not run."],
    ["cancelled", "This action was cancelled."],
    ["stale_request", "The action has changed. Please try again."],
    ["rejected", "This action was not authorized and was not run."],
    ["approved", "Approved"],
  ];
  const container = document.createElement("div");
  const root = createRoot(container);
  try {
    for (const [status, message] of cases) {
      vi.mocked(listApprovalAssessments).mockResolvedValue([{ ...entry, status }]);
      await act(async () => root.render(<ApprovalAssessments run={{ id: "r", status } as StoredRun} />));
      expect(container.textContent).toContain(message);
      for (const internal of ["%", "confidence", "probability", "provider_invalid_response", "jev-fixture", "sha256:fixture", "/example/Desktop/test.txt"])
        expect(container.textContent).not.toContain(internal);
      expect(container.querySelector("details")).toBeNull();
    }
  }
  finally { act(() => root.unmount()); }
});
