import type { StreamingMarkdownWorkerResponse } from "./streamingMarkdown.worker";
import type { StreamingMarkdownBlock } from "./streamingMarkdownBlocks";
// @vitest-environment jsdom
import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { projectStreamingMarkdown } from "./streamingMarkdownBlocks";
import { useStreamingMarkdownBlocks } from "./useStreamingMarkdownBlocks";

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

/** Props-aware probe: the hook's text changes between renders. */
function mountHook<P, R>(useHook: (props: P) => R, initialProps: P) {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  let current!: R;
  function Probe({ props }: { props: P }) {
    current = useHook(props);
    return null;
  }
  act(() => {
    root.render(createElement(Probe, { props: initialProps }));
  });
  return {
    get current() {
      return current;
    },
    setProps: (props: P) => {
      act(() => {
        root.render(createElement(Probe, { props }));
      });
    },
    unmount: () => {
      act(() => root.unmount());
      container.remove();
    },
  };
}

/**
 * A worker that answers only when the test tells it to. That models the real
 * race the provisional projection exists for: characters arrive faster than the
 * parser can answer, so the UI has to fold the unparsed suffix into the last
 * block itself instead of re-parsing on the main thread.
 */
class DeferredWorker {
  static instances: DeferredWorker[] = [];
  onmessage: ((event: MessageEvent<StreamingMarkdownWorkerResponse>) => void) | null = null;
  onerror: ((event: ErrorEvent) => void) | null = null;
  posted: Array<{ id: number; live: boolean; text: string }> = [];
  terminated = false;

  constructor(_url: string | URL, _options?: WorkerOptions) {
    DeferredWorker.instances.push(this);
  }

  postMessage(message: { id: number; live: boolean; text: string }) {
    this.posted.push(message);
  }

  terminate() {
    this.terminated = true;
  }

  respond(text: string, live: boolean) {
    const request = this.posted[this.posted.length - 1]!;
    act(() => {
      this.onmessage?.({
        data: { blocks: projectStreamingMarkdown(text, live), id: request.id, text },
      } as MessageEvent<StreamingMarkdownWorkerResponse>);
    });
  }

  /** Deliver a hand-built payload, e.g. a node shape the parser never emits. */
  respondBlocks(text: string, blocks: StreamingMarkdownBlock[]) {
    const request = this.posted[this.posted.length - 1]!;
    act(() => {
      this.onmessage?.({ data: { blocks, id: request.id, text } } as MessageEvent<StreamingMarkdownWorkerResponse>);
    });
  }
}

beforeEach(() => {
  DeferredWorker.instances = [];
  vi.stubGlobal("Worker", DeferredWorker);
});

function lastNode(block: StreamingMarkdownBlock | undefined) {
  const nodes = block?.document?.nodes ?? [];
  return { node: nodes[nodes.length - 1] as Record<string, unknown> | undefined, nodes };
}

describe("provisional suffix folding", () => {
  it.each([
    ["a paragraph", "hello", "paragraph"],
    ["a heading", "# Title", "heading"],
    ["a fenced code block", "```js\nlet x = 1;\n```", "code"],
    ["a math block", "$$\nE = mc^2\n$$", "mathBlock"],
    ["a blockquote", "> quoted line", "blockquote"],
    ["a list", "- first item", "list"],
    ["a table", "| A | B |\n| - | - |\n| 1 | 2 |", "table"],
  ])("folds an unparsed suffix into the trailing %s", (_label, markdown, type) => {
    const harness = mountHook((props: { text: string }) => useStreamingMarkdownBlocks(props.text, true), { text: markdown });
    DeferredWorker.instances[0]!.respond(markdown, true);

    const parsed = lastNode(harness.current[0]);
    expect(parsed.node?.type).toBe(type);
    expect(JSON.stringify(parsed.node)).not.toContain("SUFFIX");

    // The next characters arrive while the parser is still working on the
    // previous snapshot: no worker answer, so the hook extends the last parsed
    // node itself and the block is still the single live tail.
    const extended = `${markdown}SUFFIX`;
    harness.setProps({ text: extended });
    const provisional = lastNode(harness.current[0]);
    expect(provisional.node?.type).toBe(type);
    expect(JSON.stringify(provisional.node)).toContain("SUFFIX");
    expect(harness.current).toHaveLength(1);
    expect(harness.current[0]?.content).toBe(extended);
    harness.unmount();
  });

  it("starts a new paragraph when the trailing node cannot hold text", () => {
    const harness = mountHook((props: { text: string }) => useStreamingMarkdownBlocks(props.text, true), { text: "before\n\n---" });
    DeferredWorker.instances[0]!.respond("before\n\n---", true);
    // The splitter puts the thematic break in its own (final) block.
    const before = harness.current[harness.current.length - 1]!;
    expect(before.document?.nodes.map(node => node.type)).toEqual(["thematicBreak"]);

    harness.setProps({ text: "before\n\n---tail" });
    const after = harness.current[harness.current.length - 1]!;
    const nodes = after.document?.nodes ?? [];
    expect(nodes.map(node => node.type)).toEqual(["thematicBreak", "paragraph"]);
    expect(JSON.stringify(nodes[1])).toContain("tail");
    harness.unmount();
  });

  it("extends the last cell of a table's last row instead of adding a block", () => {
    const table = "| A | B |\n| - | - |\n| 1 | 2 |";
    const harness = mountHook((props: { text: string }) => useStreamingMarkdownBlocks(props.text, true), { text: table });
    DeferredWorker.instances[0]!.respond(table, true);

    harness.setProps({ text: `${table} extra` });
    const { node } = lastNode(harness.current[0]);
    expect(node?.type).toBe("table");
    expect(JSON.stringify(node)).toContain("extra");
    expect(harness.current).toHaveLength(1);
    harness.unmount();
  });

  it("extends a blockquote's last child rather than the blockquote itself", () => {
    const harness = mountHook((props: { text: string }) => useStreamingMarkdownBlocks(props.text, true), { text: "> first\n> second" });
    DeferredWorker.instances[0]!.respond("> first\n> second", true);

    harness.setProps({ text: "> first\n> second tail" });
    const { node } = lastNode(harness.current[0]);
    expect(node?.type).toBe("blockquote");
    const children = node?.children as Array<Record<string, unknown>>;
    expect(JSON.stringify(children[children.length - 1])).toContain("tail");
    harness.unmount();
  });

  it("replaces the folded tail with the parsed block once the worker catches up", () => {
    const harness = mountHook((props: { text: string }) => useStreamingMarkdownBlocks(props.text, true), { text: "hello" });
    const worker = DeferredWorker.instances[0]!;
    worker.respond("hello", true);
    harness.setProps({ text: "hello **world**" });
    expect(harness.current[0]?.parsed).toBe(true);

    worker.respond("hello **world**", true);
    const children = lastNode(harness.current[0]).node?.children as Array<Record<string, unknown>>;
    expect(children[1]?.type).toBe("strong");
    harness.unmount();
  });

  it("treats a worker payload with no extendable tail as a fresh paragraph", () => {
    // A malformed or older worker payload can describe a container with nothing
    // to extend. The suffix must still be shown, not dropped.
    const cases: Array<[string, Record<string, unknown>]> = [
      ["a blockquote with no children", { children: [], type: "blockquote" }],
      ["a list with no items", { items: [], type: "list" }],
      ["a table with no rows or headers", { headers: [], rows: [], type: "table" }],
    ];
    for (const [label, node] of cases) {
      const harness = mountHook((props: { text: string }) => useStreamingMarkdownBlocks(props.text, true), { text: "x" });
      DeferredWorker.instances[DeferredWorker.instances.length - 1]!.respondBlocks("x", [{
        content: "x",
        document: { nodes: [node], raw: "x", references: [] },
        live: true,
        parsed: true,
        start: 0,
      } as unknown as StreamingMarkdownBlock]);
      harness.setProps({ text: "xtail" });
      const nodes = harness.current[0]?.document?.nodes ?? [];
      expect(nodes, label).toHaveLength(2);
      expect(nodes[1]?.type, label).toBe("paragraph");
      expect(JSON.stringify(nodes[1]), label).toContain("tail");
      harness.unmount();
    }
  });

  it("extends a loose list item's last block, and falls back when it cannot", () => {
    const paragraph = { children: [{ text: "first", type: "text" }], type: "paragraph" };

    const extendable = mountHook((props: { text: string }) => useStreamingMarkdownBlocks(props.text, true), { text: "x" });
    DeferredWorker.instances[DeferredWorker.instances.length - 1]!.respondBlocks("x", [{
      content: "x",
      document: {
        nodes: [{ items: [{ blocks: [paragraph, { children: [{ text: "second", type: "text" }], type: "paragraph" }], children: [] }], type: "list" }],
        raw: "x",
        references: [],
      },
      live: true,
      parsed: true,
      start: 0,
    } as unknown as StreamingMarkdownBlock]);
    extendable.setProps({ text: "xtail" });
    const list = extendable.current[0]?.document?.nodes[0] as Record<string, unknown>;
    const items = list.items as Array<Record<string, unknown>>;
    const blocks = items[0]?.blocks as Array<Record<string, unknown>>;
    expect(blocks[1]?.type).toBe("paragraph");
    expect(JSON.stringify(blocks[1])).toContain("tail");
    extendable.unmount();

    // The same shape whose last block cannot hold text starts a new paragraph.
    const unextendable = mountHook((props: { text: string }) => useStreamingMarkdownBlocks(props.text, true), { text: "x" });
    DeferredWorker.instances[DeferredWorker.instances.length - 1]!.respondBlocks("x", [{
      content: "x",
      document: {
        nodes: [{ items: [{ blocks: [{ type: "thematicBreak" }], children: [] }], type: "list" }],
        raw: "x",
        references: [],
      },
      live: true,
      parsed: true,
      start: 0,
    } as unknown as StreamingMarkdownBlock]);
    unextendable.setProps({ text: "xtail" });
    const nodes = unextendable.current[0]?.document?.nodes ?? [];
    expect(nodes).toHaveLength(2);
    expect(nodes[1]?.type).toBe("paragraph");
    expect(JSON.stringify(nodes[1])).toContain("tail");
    unextendable.unmount();
  });

  it("extends a header-only table's last header cell", () => {
    const harness = mountHook((props: { text: string }) => useStreamingMarkdownBlocks(props.text, true), { text: "| A | B |\n| - | - |" });
    DeferredWorker.instances[DeferredWorker.instances.length - 1]!.respondBlocks("| A | B |\n| - | - |", [{
      content: "| A | B |\n| - | - |",
      document: {
        nodes: [{
          headers: [[{ text: "A", type: "text" }], [{ text: "B", type: "text" }]],
          rows: [],
          type: "table",
        }],
        raw: "| A | B |\n| - | - |",
        references: [],
      },
      live: true,
      parsed: true,
      start: 0,
    } as unknown as StreamingMarkdownBlock]);
    harness.setProps({ text: "| A | B |\n| - | - |tail" });
    const table = harness.current[0]?.document?.nodes[0] as Record<string, unknown>;
    expect(table.type).toBe("table");
    const headers = table.headers as Array<Array<Record<string, unknown>>>;
    expect(JSON.stringify(headers[headers.length - 1])).toContain("tail");
    harness.unmount();
  });

  it("falls back to a new paragraph when a blockquote's last child cannot hold text", () => {
    // A blockquote whose final child is a thematic break has nowhere to put the
    // suffix, so the document-level fallback takes over.
    const harness = mountHook((props: { text: string }) => useStreamingMarkdownBlocks(props.text, true), { text: "x" });
    DeferredWorker.instances[DeferredWorker.instances.length - 1]!.respondBlocks("x", [{
      content: "x",
      document: {
        nodes: [{ children: [{ type: "thematicBreak" }], type: "blockquote" }],
        raw: "x",
        references: [],
      },
      live: true,
      parsed: true,
      start: 0,
    } as unknown as StreamingMarkdownBlock]);
    harness.setProps({ text: "xtail" });
    const nodes = harness.current[0]?.document?.nodes ?? [];
    expect(nodes).toHaveLength(2);
    expect(nodes[1]?.type).toBe("paragraph");
    expect(JSON.stringify(nodes[1])).toContain("tail");
    harness.unmount();
  });

  it("ignores a response from a worker that has already been replaced", () => {
    const harness = mountHook((props: { text: string }) => useStreamingMarkdownBlocks(props.text, true), { text: "A" });
    const first = DeferredWorker.instances[0]!;
    act(() => first.onerror?.({} as ErrorEvent));
    harness.setProps({ text: "AB" });
    const second = DeferredWorker.instances[1]!;
    expect(second).toBeDefined();

    // A distinctive document only this payload could produce.
    const payload = () => ({
      content: "AB",
      document: { nodes: [{ children: [{ text: "H", type: "text" }], type: "heading" }], raw: "AB", references: [] },
      live: true,
      parsed: true,
      start: 0,
    } as unknown as StreamingMarkdownBlock);

    // The dead worker's late answer must not replace the live projection…
    first.respondBlocks("AB", [payload()]);
    expect(harness.current[0]?.document?.nodes[0]?.type).not.toBe("heading");

    // …while the replacement worker's answer is applied.
    second.respondBlocks("AB", [payload()]);
    expect(harness.current[0]?.document?.nodes[0]?.type).toBe("heading");
    harness.unmount();
  });

  it("shows every character even while the worker is several snapshots behind", () => {
    let text = "first";
    const harness = mountHook((props: { text: string }) => useStreamingMarkdownBlocks(props.text, true), { text });
    DeferredWorker.instances[0]!.respond(text, true);
    for (const chunk of [" second", " third", " fourth"]) {
      text += chunk;
      harness.setProps({ text });
    }
    expect(harness.current[0]?.content).toBe(text);
    expect(JSON.stringify(lastNode(harness.current[0]).node)).toContain("fourth");
    harness.unmount();
  });
});

describe("streaming projection fallbacks", () => {
  it("starts over when the document is replaced rather than extended", () => {
    const harness = mountHook((props: { text: string }) => useStreamingMarkdownBlocks(props.text, true), { text: "first paragraph" });
    DeferredWorker.instances[0]!.respond("first paragraph", true);
    expect(harness.current[0]?.document?.nodes).toHaveLength(1);

    harness.setProps({ text: "an entirely different document" });
    expect(harness.current).toHaveLength(1);
    expect(harness.current[0]?.content).toBe("an entirely different document");
    harness.unmount();
  });

  it("shows raw source without parsing when there is no Worker at all", () => {
    vi.stubGlobal("Worker", undefined);
    const text = "x".repeat(9000);
    const harness = mountHook(() => useStreamingMarkdownBlocks(text, true), {});
    // Above the synchronous fallback bound the source is shown as one paragraph
    // instead of being parsed on the UI thread.
    expect(harness.current[0]?.document?.nodes).toEqual([{ type: "paragraph", children: [{ type: "text", text }] }]);
    harness.unmount();

    const small = mountHook(() => useStreamingMarkdownBlocks("**bold** text", true), {});
    const node = small.current[0]?.document?.nodes[0];
    const children = node?.type === "paragraph" ? node.children : undefined;
    expect(children?.[0]?.type).toBe("strong");
    small.unmount();
  });

  it("returns no blocks for empty source and one block for a finished stream", () => {
    expect(projectStreamingMarkdown("", true)).toEqual([]);
    expect(projectStreamingMarkdown("plain text", false)).toHaveLength(1);
  });

  it("keeps a static message static, then starts projecting once the thread streams", () => {
    const harness = mountHook((props: { live: boolean }) => useStreamingMarkdownBlocks("static", props.live), { live: false });
    expect(harness.current).toEqual([{ content: "static", live: false, start: 0 }]);

    // A message that turns out to be part of a stream is projected from then
    // on; the sticky flag means it is never downgraded again.
    harness.setProps({ live: true });
    expect(harness.current).toHaveLength(1);
    expect(harness.current[0]?.document?.raw).toBe("static");
    harness.setProps({ live: false });
    expect(harness.current[0]?.live).toBe(false);
    harness.unmount();
  });
});
