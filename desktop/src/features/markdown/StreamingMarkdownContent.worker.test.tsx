// @vitest-environment jsdom
import type { StreamingMarkdownWorkerRequest } from "./streamingMarkdown.worker";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { StreamingMarkdownContent } from "./MarkdownContent";
import { parseFutureMarkdown } from "./parseFutureMarkdown";
import { projectStreamingMarkdown } from "./streamingMarkdownBlocks";

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

vi.mock("./parseFutureMarkdown", async original => ({
  ...await original<typeof import("./parseFutureMarkdown")>(),
  parseFutureMarkdown: vi.fn((await original<typeof import("./parseFutureMarkdown")>()).parseFutureMarkdown),
}));

class ParserWorker {
  static current: ParserWorker;
  onmessage: Worker["onmessage"] = null;
  pending: StreamingMarkdownWorkerRequest | null = null;
  terminated = false;
  constructor() {
    ParserWorker.current = this;
  }

  postMessage(request: StreamingMarkdownWorkerRequest) {
    this.pending = request;
  }

  finish() {
    const request = this.pending!;
    const response = structuredClone({ ...request, blocks: projectStreamingMarkdown(request.text, request.live) });
    this.onmessage?.call(this as unknown as Worker, { data: response } as MessageEvent);
  }

  terminate() {
    this.terminated = true;
  }
}

afterEach(() => {
  vi.unstubAllGlobals();
  vi.clearAllMocks();
});

it("renders worker ASTs, displays pending source, and finalizes without any UI-thread parse", () => {
  vi.stubGlobal("Worker", ParserWorker);
  const host = document.createElement("div");
  const root = createRoot(host);
  let text = "| A | B |\n| - | - |\n| **first** | second |";
  try {
    act(() => root.render(<StreamingMarkdownContent content={text} live />));
    expect(host.textContent).toBe(text);
    act(() => ParserWorker.current.finish());
    expect(host.querySelectorAll("tbody tr")).toHaveLength(1);
    expect(host.querySelector("strong")?.textContent).toBe("first");
    text += "\n| next | row |";
    act(() => root.render(<StreamingMarkdownContent content={text} live />));
    expect(host.textContent).toContain("| next | row |");
    act(() => ParserWorker.current.finish());
    expect(host.querySelectorAll("tbody tr")).toHaveLength(2);
    act(() => root.render(<StreamingMarkdownContent content={text} live={false} />));
    act(() => ParserWorker.current.finish());
    expect(ParserWorker.current.terminated).toBe(true);
    expect(host.querySelector("[data-streamed-block]")).toBeNull();
    expect(parseFutureMarkdown).not.toHaveBeenCalled();
  }
  finally {
    act(() => root.unmount());
  }
});

it("projects the same rich nodes as static parsing for whole-document and split Markdown", async () => {
  const { parseFutureMarkdown: parse } = await import("@future-os/markdown");
  for (const text of [
    "| A | B |\n| - | - |\n| **x** | y |",
    "Read [docs][id].\n\n[id]: https://example.com",
    "# Title\n\nParagraph\n\n```ts\nconst x = 1;\n```",
    "\\[\nx^2 + y^2\n\\]",
  ]) {
    const blocks = projectStreamingMarkdown(text, false);
    expect(blocks.map(block => block.content).join("")).toBe(text);
    expect(blocks.flatMap(block => block.document!.nodes)).toEqual(parse(text).nodes);
  }
});
