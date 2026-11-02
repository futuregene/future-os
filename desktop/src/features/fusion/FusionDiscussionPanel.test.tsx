// @vitest-environment jsdom
import type { FusionDiscussion, FusionStep } from "@future-os/thread-projection";
import type { Root } from "react-dom/client";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import fixture from "../../../../scripts/screenshots/fixtures/fusion.json";
import { readFusion } from "../../integrations/fusion/queries";
import { FusionDiscussionPanel } from "./FusionDiscussionPanel";
import { FusionStepBody } from "./FusionStepBody";

const mocks = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("../../integrations/tauri/invoke", () => ({ invokeCommand: mocks.invoke }));
vi.mock("../markdown/MarkdownContent", () => ({ MarkdownContent: ({ content }: { content: string }) => <div>{content}</div> }));
(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;
let root: Root;
let container: HTMLDivElement;

beforeEach(() => {
  mocks.invoke.mockReset();
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});
afterEach(() => {
  act(() => root.unmount());
  container.remove();
});
function discussion(): FusionDiscussion {
  return JSON.parse(JSON.stringify(fixture.discussion)) as FusionDiscussion;
}
async function render(runId = "r1", toolCallId = "t1") {
  await act(async () => root.render(<FusionDiscussionPanel runId={runId} toolCallId={toolCallId} onBack={() => {}} />));
}
async function click(text: string) {
  const button = [...container.querySelectorAll("button")].find(button => button.textContent?.includes(text));
  expect(button).toBeTruthy();
  await act(async () => button!.click());
}

it("reads the selected parent and pages a public response using the server byte cursor", async () => {
  const data = discussion();
  const step = data.steps![0]!;
  const answer = "Public answer 中文 with more detail than the stored preview.";
  const transcript = JSON.stringify([{ role: "assistant", content: [{ type: "text", text: answer }] }]);
  const split = transcript.indexOf("Public") + 3;
  const cursor = new TextEncoder().encode(transcript.slice(0, split)).length;
  const total = new TextEncoder().encode(transcript).length;
  mocks.invoke.mockImplementation(async (_command, input) => {
    if (!input.stepId)
      return data;
    return { ...step, transcript: input.offset === 0 ? transcript.slice(0, split) : transcript.slice(split), next_offset: input.offset === 0 ? cursor : total, total_bytes: total } satisfies FusionStep;
  });
  await render();
  expect(mocks.invoke).toHaveBeenCalledWith("get_fusion", { runId: "r1", toolCallId: "t1" });
  await click("future/deepseek-flash");
  expect(container.textContent).toContain(step.output);
  expect(mocks.invoke).toHaveBeenCalledWith("get_fusion", { runId: "r1", toolCallId: "t1", stepId: step.id, offset: 0 });
  await click("Read more");
  expect(mocks.invoke).toHaveBeenLastCalledWith("get_fusion", { runId: "r1", toolCallId: "t1", stepId: step.id, offset: cursor });
  expect(container.querySelector("[data-fusion-public-output]")?.textContent).toContain(answer);
});

it("retains failed attempts and public output without presenting a stale conclusion", async () => {
  const data = discussion();
  data.run!.status = "failed";
  data.run!.error = "Moderator unavailable";
  data.steps![0]!.status = "failed";
  data.steps![0]!.error = "Original failure";
  mocks.invoke.mockImplementation(async (_command, input) => input.stepId ? data.steps![0] : data);
  await render();
  expect(container.textContent).toContain("did not produce a final conclusion");
  expect(container.textContent).not.toContain("Moderator unavailable");
  expect(container.querySelector("h3")?.textContent).toBe("Discussion");
  await click("future/deepseek-flash");
  expect(container.textContent).not.toContain("Original failure");
  expect(container.textContent).toContain(data.steps![0]!.output);
});

it("ignores a late response from a previously selected parent", async () => {
  let finishOld!: (data: FusionDiscussion) => void;
  const current = discussion();
  current.run!.input.question = "Current decision";
  mocks.invoke.mockImplementation((_command, input) => {
    if (input.runId === "old") {
      return new Promise((resolve) => {
        finishOld = resolve;
      });
    }
    return Promise.resolve(current);
  });
  await render("old");
  await render("current");
  const old = discussion();
  old.run!.input.question = "Obsolete decision";
  await act(async () => finishOld(old));
  expect(container.textContent).toContain("Current decision");
  expect(container.textContent).not.toContain("Obsolete decision");
});

it("keeps failed remote reads on the original host without a local fallback", async () => {
  const resolver = vi.fn(async () => null);
  await expect(readFusion({ runId: "remote-run", toolCallId: "remote-call", stepId: "s1", offset: 32768 }, resolver)).rejects.toThrow();
  expect(resolver).toHaveBeenCalledWith({ runId: "remote-run", toolCallId: "remote-call", stepId: "s1", offset: 32768, fusion: true });
  expect(mocks.invoke).not.toHaveBeenCalled();
});

it("does not expose internal preflight or the input-context section", async () => {
  const data = discussion();
  data.run!.status = "running";
  data.run!.phase = "preflight";
  data.run!.result = null;
  data.steps = [{ ...data.steps![0]!, id: "preflight", phase: "preflight", participant: "moderator", status: "running" }];
  mocks.invoke.mockResolvedValue(data);
  await render();
  expect(container.textContent).not.toContain("Checking main model");
  expect(container.textContent).not.toContain("Background and constraints");
  expect(container.textContent).toContain(data.run!.input.question);
});

it("renders sourced mapping and explicit silence without moderator omissions", async () => {
  const step = { ...discussion().steps![0]!, phase: "issue_mapping", output: JSON.stringify({
    consensus: [{ point: "Keep data", sources: [{ participant: "P1", quote: "Preserve existing records" }] }],
    issues: [{ topic: "Storage choice", stances: [
      { participant: "P1", status: "stated", position: "SQLite", quote: "Use SQLite" },
      { participant: "P2", status: "not_mentioned", position: "", quote: "" },
    ] }],
    excluded: [{ participant: "P1", quote: "Delete records", reason: "Outside user scope", basis: "Preserve user data" }],
    blind_spots: ["An old moderator omission"],
  }) };
  mocks.invoke.mockResolvedValue({ ...step, transcript: "", next_offset: 0, total_bytes: 0 });
  await act(async () => root.render(<FusionStepBody step={step} runId="r1" toolCallId="t1" />));
  const output = container.querySelector("[data-fusion-public-output]")?.textContent;
  expect(output).toContain("Keep data");
  expect(output).toContain("Preserve existing records");
  expect(output).toContain("Not mentioned");
  expect(output).toContain("Outside user scope");
  expect(output).toContain("Preserve user data");
  expect(output).not.toContain("An old moderator omission");
});

it("renders own revisions and separates proposals from reported facts", async () => {
  const step = { ...discussion().steps![0]!, phase: "reviewing", output: JSON.stringify({
    participant: "P1",
    current_answer: "Current answer to the topic",
    responses: [{ participant: "P2", quote: "Use files", decision: "insufficient_information", reason: "Missing rationale" }],
    self_review: [{ original_quote: "Prefer SQLite", decision: "revise", updated_position: "Investigate storage requirements first", reason: "Need workload evidence" }],
    mapping_corrections: [{ summary_quote: "Always SQLite", source_quote: "Prefer SQLite", correction: "Preference is conditional", reason: "The map overstated certainty" }],
    additions: [{ kind: "proposal", content: "Measure the workload", evidence: [] }, { kind: "unverified_claim", content: "Latency may differ", evidence: [] }],
  }) };
  mocks.invoke.mockResolvedValue({ ...step, transcript: "", next_offset: 0, total_bytes: 0 });
  await act(async () => root.render(<FusionStepBody step={step} runId="r1" toolCallId="t1" />));
  const output = container.querySelector("[data-fusion-public-output]")?.textContent;
  expect(output).toContain("Current answer to the topic");
  expect(output).toContain("Insufficient information");
  expect(output).toContain("Investigate storage requirements first");
  expect(output).toContain("Preference is conditional");
  expect(output).toContain("New proposal");
  expect(output).toContain("Unverified claim");
});
