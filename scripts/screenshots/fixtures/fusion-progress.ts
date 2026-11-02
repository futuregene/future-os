import type { FusionDiscussion } from "../../../packages/thread-projection/src/fusion";
import fixture from "./fusion.json";
/** Synthetic states; never includes a user's original discussion text. */
export function fusionProgressFixture(mode: string | null): FusionDiscussion {
  const data: FusionDiscussion = JSON.parse(JSON.stringify(fixture.discussion));
  if (mode === "partial") {
    data.run!.completeness = "partial";
    for (const step of data.steps ?? []) {
      if (step.participant === "P2") {
        step.status = "failed";
        step.error = "[OUTPUT_LIMIT] finish_length";
      }
    }
  }
  if (mode === "running") {
    data.run!.status = "running";
    data.run!.phase = "reviewing";
    data.steps = data.steps?.filter(step => step.phase !== "synthesizing");
    for (const step of data.steps ?? []) {
      if (step.phase === "reviewing") step.status = "running";
    }
  }
  if (mode === "mapping") {
    data.run!.status = "running";
    data.run!.phase = "issue_mapping";
    data.run!.result = null;
    data.steps = data.steps?.filter(step => step.phase === "analyzing" || step.phase === "issue_mapping");
    for (const step of data.steps ?? []) {
      if (step.phase === "issue_mapping") {
        step.status = "running";
        step.output = "";
      }
    }
  }
  if (mode === "failed") {
    data.run!.status = "failed";
    data.run!.result = null;
    data.run!.error = "主持模型无法生成最终结论；已有公开发言已保留。";
    for (const step of data.steps ?? []) {
      if (step.phase === "synthesizing") {
        step.status = "failed";
        step.error = "HTTP 503";
        step.output = "";
      }
    }
  }
  return data;
}
