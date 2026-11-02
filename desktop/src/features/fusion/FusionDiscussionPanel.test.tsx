// @vitest-environment jsdom
import type { FusionDiscussion, FusionStep } from "@future-os/thread-projection";
import type { Root } from "react-dom/client";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import fixture from "../../../../scripts/screenshots/fixtures/fusion.json";
import { readFusion } from "../../integrations/fusion/queries";
import { FusionDiscussionPanel } from "./FusionDiscussionPanel";

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
  expect(container.textContent).toContain("Moderator unavailable");
  expect(container.querySelector("h3")?.textContent).toBe("Discussion");
  await click("future/deepseek-flash");
  expect(container.textContent).toContain("Original failure");
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
