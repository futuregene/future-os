import type { AgentMessage } from "@future-os/thread-projection";
import { describe, expect, it } from "vitest";
import { mayContainThreadSearch } from "./threadSearchCache";

function message(content: string): AgentMessage {
  return { id: "reply", role: "assistant", content, status: "complete" } as AgentMessage;
}

describe("cached search prefilter", () => {
  it("finds phrases split by Markdown formatting", () => {
    expect(mayContainThreadSearch(message("a **formatted** phrase"), "A formatted phrase")).toBe(true);
    expect(mayContainThreadSearch(message("```ts\nconst answer = 42;\n```"), "answer = 42")).toBe(true);
  });

  it("leaves unrelated cached text unmounted", () => {
    expect(mayContainThreadSearch(message("unrelated text"), "needle")).toBe(false);
    expect(mayContainThreadSearch(message("needle"), "")).toBe(false);
  });

  it("defers dynamic activity labels to the actual rendered content", () => {
    expect(mayContainThreadSearch({ ...message(""), activityItems: [{ id: "tool", kind: "shell", status: "completed" }] }, "commands")).toBe(true);
  });
});
