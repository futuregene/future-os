import { describe, expect, it } from "vitest";
import { fusionProgress, type FusionDiscussion, type FusionStep } from "./fusion";
import { fusionSettingsIssue, newFusionParticipant } from "./fusionSettings";
function step(phase: string, model: string, participant: string, status: string, at: number, attempt = 0, error: string | null = null): FusionStep {
  return { id: `${phase}-${participant}-${attempt}`, phase, model, participant, status, attempt, error, input: "", output: "", session_id: "", agent_run_id: "", started_at: at, usage: null };
}
const deepseek = "future/deepseek-flash";
const glm = "future/glm-5.3-flash";
const qwen = "future/qwen3.7-plus";
function discussion(steps: FusionStep[], phase = "synthesizing", status = "completed", completeness = "full"): FusionDiscussion {
  return { run: { id: "f", phase, status, completeness, input: { question: "A or B?", goal: "choose", background: "facts", constraints: [] } }, steps };
}
describe("Fusion progress", () => {
  it("keeps identical models in separate roles and supports two to five participants", () => {
    for (const count of [2, 5]) {
      const profile = { ...newFusionParticipant(), model: deepseek, thinking_level: "medium" };
      const value = discussion([step("analyzing", deepseek, "P1", "failed", 1), step("analyzing", deepseek, "P2", "running", 2)], "analyzing", "running");
      value.run!.configuration = { version: 2, moderator: profile, panel: Array.from({ length: count }, () => ({ ...profile, thinking_level: "low" })), step_timeout_secs: 300, run_timeout_secs: 2700 };
      expect(fusionSettingsIssue(value.run!.configuration)).toBeNull();
      const progress = fusionProgress(value);
      expect(progress.models).toHaveLength(count + 1);
      expect(progress.models.find(model => model.id === "moderator")?.status).toBe("waiting");
      expect(progress.models.find(model => model.id === "P1")?.status).toBe("failed");
      expect(progress.models.find(model => model.id === "P2")?.status).toBe("thinking");
      value.run!.configuration.moderator = { ...profile, model: "" };
      expect(fusionSettingsIssue(value.run!.configuration)).toBe("moderatorRequired");
    }
  });
  it("shows stages and exactly one row per model", () => {
    const value = fusionProgress(discussion([
      step("analyzing", deepseek, "P1", "completed", 1), step("reviewing", deepseek, "P1", "completed", 2), step("synthesizing", deepseek, "moderator", "running", 3),
    ], "synthesizing", "running"));
    expect(value.completedStages).toBe(3);
    expect(value.models).toHaveLength(3);
    expect(value.models[0].status).toBe("thinking");
  });
  it("retains partial participants and distinguishes output limit from timeout", () => {
    const value = fusionProgress(discussion([
      step("analyzing", glm, "P2", "failed", 1, 0, "[OUTPUT_LIMIT] finish_length"),
      step("analyzing", glm, "P2", "failed", 2, 1, "Fusion model timed out"),
      step("reviewing", glm, "P2", "failed", 3, 0, "[OUTPUT_LIMIT] finish_length"),
      step("synthesizing", deepseek, "moderator", "completed", 4), step("reviewing", qwen, "P3", "completed", 3),
    ], "synthesizing", "completed", "partial"));
    expect(value.completedStages).toBe(4);
    expect(value.partial).toBe(true);
    expect(value.models.find(model => model.model === glm)?.status).toBe("output_limited");
    const timeout = fusionProgress(discussion([step("analyzing", glm, "P2", "failed", 2, 1, "Fusion model timed out")], "analyzing", "failed"));
    expect(timeout.models.find(model => model.model === glm)?.status).toBe("timed_out");
  });
  it("a successful retry replaces its failed attempt without masking another failed phase", () => {
    const recovered = [step("analyzing", glm, "P2", "failed", 1), step("analyzing", glm, "P2", "completed", 2, 1)];
    expect(fusionProgress(discussion(recovered)).models.find(model => model.model === glm)?.status).toBe("completed");
    expect(fusionProgress(discussion([...recovered, step("reviewing", glm, "P2", "failed", 3), step("synthesizing", deepseek, "moderator", "completed", 4)], "synthesizing", "completed", "partial")).models.find(model => model.model === glm)?.status).toBe("failed");
  });
  it("does not reuse completed analysis as the status of a pending review", () => {
    const value = fusionProgress(discussion([step("analyzing", glm, "P2", "completed", 1)], "reviewing", "running"));
    expect(value.models.find(model => model.model === glm)?.status).toBe("waiting");
  });
  it("keeps prior-stage failure visible after a later stage succeeds", () => {
    const value = fusionProgress(discussion([
      step("analyzing", glm, "P2", "failed", 1),
      step("reviewing", glm, "P2", "completed", 2),
      step("synthesizing", deepseek, "moderator", "completed", 3),
    ], "synthesizing", "completed", "partial"));
    expect(value.models.find(model => model.model === glm)?.status).toBe("partial");
  });
  it("fences thinking rows when cancelled or interrupted", () => {
    const steps = [step("reviewing", qwen, "P3", "running", 1)];
    expect(fusionProgress(discussion(steps, "reviewing", "cancelled")).models.find(model => model.model === qwen)?.status).toBe("cancelled");
    expect(fusionProgress(discussion(steps, "reviewing", "running"), "failed").models.find(model => model.model === qwen)?.status).toBe("interrupted");
  });
  it("uses durable coverage for skipped phases and does not invent a conclusion", () => {
    const value = discussion([step("analyzing", glm, "P2", "skipped", 1)], "synthesizing", "failed", "none");
    value.run!.report = { version: 1, strategy: "single_advisor", completed_phases: ["analyzing"], skipped_phases: ["issue_mapping", "reviewing"], analysis_participants: ["P1"], review_participants: [], degradations: [{ code: "moderator_failed", phase: "synthesizing", kind: "timeout" }], available_results: [{ step_id: "s", phase: "analyzing", participant: "P1", model: deepseek, text: "public evidence", truncated: false }] };
    const progress = fusionProgress(value);
    expect(progress.stages).toEqual(["completed", "skipped", "skipped", "failed"]);
    expect(progress.completedStages).toBe(1);
    expect(progress.models.find(model => model.model === glm)?.status).toBe("skipped");
    expect(progress.notices).toEqual(["noConclusion", "moderatorFailed", "materialsRetained"]);
  });
  it("uses typed failures over ambiguous timeout wording and retains a failed first round", () => {
    const limited = step("reviewing", glm, "P2", "failed", 2, 0, "upstream timeout");
    limited.failure_kind = "budget_reserved";
    expect(fusionProgress(discussion([limited])).models.find(model => model.model === glm)?.status).toBe("skipped");
    const failed = step("analyzing", glm, "P2", "failed", 1);
    failed.failure_kind = "timeout";
    expect(fusionProgress(discussion([failed])).models.find(model => model.model === glm)?.status).toBe("timed_out");
  });

});
