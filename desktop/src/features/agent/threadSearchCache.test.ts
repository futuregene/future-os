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

  it("reads the text segments of a segmented reply", () => {
    // boundary: a projected reply carries `segments` instead of a single content
    // string. When every segment is text (a non-text one returns early above), the
    // prefilter must search the concatenation of those segment texts rather than
    // `content` - which for a segmented reply is not the rendered text.
    const segmented = {
      id: "reply",
      content: "",
      role: "assistant",
      segments: [
        { key: "s1", kind: "text", text: "the needle is " },
        { key: "s2", kind: "text", text: "inside the segments" },
      ],
      status: "complete",
    } as unknown as AgentMessage;

    expect(mayContainThreadSearch(segmented, "needle is inside")).toBe(true);
    expect(mayContainThreadSearch(segmented, "absent")).toBe(false);
  });

  it("mounts a reply that carries an application reference", () => { // boundary: the parsed document can hold references even when the needle is
    // absent from the flattened text, so a miss on the text alone must not exclude
    // the row. Without this arm a referenced row would be filtered out and the
    // search would silently skip it.
    const withReference = message("see [c](docs/c.md) for details");

    expect(mayContainThreadSearch(withReference, "details")).toBe(true);
    // The needle matches nothing in the text, so only the reference keeps the row.
    expect(mayContainThreadSearch(withReference, "not-present-anywhere")).toBe(true);
  });

  it("flattens a code block's code when the needle misses the raw text", () => {
    // This is the branch I had flagged in §8 as possibly dead, and my argument for
    // that was wrong: I reasoned that a code node's `code` is a substring of `text`,
    // so the raw-text check at `:20` would have matched first. That conflates two
    // different things - `textLeaves` is reached precisely when the needle is ABSENT
    // from the raw text, and at that point ANY code node is still visited. So the
    // `typeof node.code === "string"` branch runs for any assistant reply containing
    // a fenced block and a non-matching needle, which is an ordinary search miss.
    const fenced = message("```ts\nconst answer = 42;\n```");

    // The needle is absent from the raw markdown, yet no reference exists either, so
    // the walk continues into the parsed nodes and returns the code's own text.
    expect(mayContainThreadSearch(fenced, "zzz-absent")).toBe(false);
    // The flattened code is what the walk compares, so a needle that only matches
    // inside the code node still finds it there.
    expect(mayContainThreadSearch(fenced, "answer = 42")).toBe(true);
  });

  it("returns early for a user message whose text does not match", () => {
    // boundary: user messages are plain text (never markdown), so a miss on their
    // raw text is definitive - there is nothing to flatten. The early return is what
    // keeps a user row out of the candidate list instead of walking a parsed
    // document that upstream rendering never produces for this role.
    const prompt = { id: "u1", role: "user", content: "a plain question", status: "complete" } as AgentMessage;

    expect(mayContainThreadSearch(prompt, "zzz-absent")).toBe(false);
    expect(mayContainThreadSearch(prompt, "plain question")).toBe(true);
  });
});
