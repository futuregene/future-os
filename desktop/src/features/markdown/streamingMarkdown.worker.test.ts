import type { StreamingMarkdownWorkerRequest, StreamingMarkdownWorkerResponse } from "./streamingMarkdown.worker";
// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const posted: StreamingMarkdownWorkerResponse[] = [];

beforeEach(async () => {
  posted.length = 0;
  vi.stubGlobal("postMessage", (message: StreamingMarkdownWorkerResponse) => {
    posted.push(message);
  });
  // Importing the module installs the scope's message handler, exactly as the
  // real worker entry point does.
  await import("./streamingMarkdown.worker");
});

afterEach(() => {
  vi.unstubAllGlobals();
});

function deliver(request: StreamingMarkdownWorkerRequest) {
  const scope = globalThis as unknown as { onmessage: ((event: MessageEvent<StreamingMarkdownWorkerRequest>) => void) | null };
  scope.onmessage?.({ data: request } as MessageEvent<StreamingMarkdownWorkerRequest>);
}

describe("streaming markdown worker entry", () => {
  it("projects a request into blocks split at top-level boundaries", () => {
    deliver({ id: 7, live: true, text: "# Title\n\nbody text" });
    expect(posted).toHaveLength(1);
    const response = posted[0]!;
    expect(response.id).toBe(7);
    expect(response.text).toBe("# Title\n\nbody text");
    expect(response.blocks.map(block => block.content)).toEqual(["# Title\n\n", "body text"]);
    // Only the growing tail is live; the completed block is frozen.
    expect(response.blocks.map(block => block.live)).toEqual([false, true]);
    expect(response.blocks[1]?.document?.nodes[0]).toMatchObject({
      children: [{ text: "body text", type: "text" }],
      type: "paragraph",
    });
    // The first parse in a process loads the whole remark pipeline; on a busy
    // machine that alone can take longer than the default timeout.
  }, 30_000);

  it("answers each request with its own id and live flag", () => {
    deliver({ id: 1, live: true, text: "one" });
    deliver({ id: 2, live: false, text: "one two" });
    expect(posted.map(response => response.id)).toEqual([1, 2]);
    const blocks = posted[1]?.blocks ?? [];
    expect(blocks[blocks.length - 1]?.live).toBe(false);
    // The projector keeps its previous document, so the second answer is a
    // prefix-extension of the first.
    expect(posted[1]?.text).toBe("one two");
  }, 30_000);

  it("answers an empty request with no blocks", () => {
    deliver({ id: 3, live: true, text: "" });
    expect(posted[0]?.blocks).toEqual([]);
  }, 30_000);

  it("handles a very long single-line request without dropping it", () => {
    const text = "x".repeat(50_000);
    deliver({ id: 4, live: true, text });
    expect(posted[0]?.blocks).toHaveLength(1);
    expect(posted[0]?.blocks[0]?.content).toBe(text);
  }, 30_000);
});
