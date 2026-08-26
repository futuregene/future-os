import type { StreamingMarkdownWorkerResponse } from "./streamingMarkdown.worker";
// @vitest-environment jsdom
import { act } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { renderHook } from "../../test/renderHook";
import { useStreamingMarkdownBlocks } from "./useStreamingMarkdownBlocks";

interface PostedRequest {
  id: number;
  live: boolean;
  text: string;
}

class FakeWorker {
  static instances: FakeWorker[] = [];
  onmessage: ((event: MessageEvent<StreamingMarkdownWorkerResponse>) => void) | null = null;
  onerror: ((event: ErrorEvent) => void) | null = null;
  posted: PostedRequest[] = [];
  terminated = false;

  constructor(_url: string | URL, _options?: WorkerOptions) {
    FakeWorker.instances.push(this);
  }

  postMessage(message: PostedRequest) {
    this.posted.push(message);
  }

  terminate() {
    this.terminated = true;
  }

  emit(response: StreamingMarkdownWorkerResponse) {
    this.onmessage?.({
      data: response,
    } as MessageEvent<StreamingMarkdownWorkerResponse>);
  }

  emitError() {
    this.onerror?.({} as ErrorEvent);
  }
}

beforeEach(() => {
  FakeWorker.instances = [];
  vi.stubGlobal("Worker", FakeWorker);
});

function respond(worker: FakeWorker, response: StreamingMarkdownWorkerResponse) {
  act(() => worker.emit(response));
}

function fail(worker: FakeWorker) {
  act(() => worker.emitError());
}

describe("useStreamingMarkdownBlocks", () => {
  it("returns a single static block for a never-live thread", () => {
    const h = renderHook(() => useStreamingMarkdownBlocks("hello", false));
    expect(h.current).toEqual([{ content: "hello", live: false, start: 0 }]);
    expect(FakeWorker.instances).toHaveLength(0);
    h.unmount();
  });

  it("projects synchronously when Worker is unavailable", () => {
    vi.stubGlobal("Worker", undefined);
    const h = renderHook(() => useStreamingMarkdownBlocks("A", true));
    expect(h.current).toEqual([{ content: "A", live: true, start: 0 }]);
    h.unmount();
  });

  it("creates a worker and posts the initial request", () => {
    const h = renderHook(() => useStreamingMarkdownBlocks("A", true));
    expect(FakeWorker.instances).toHaveLength(1);
    expect(FakeWorker.instances[0]!.posted).toEqual([{ id: 1, live: true, text: "A" }]);
    // Unchanged text returns the current block projection unchanged.
    expect(h.current).toEqual([{ content: "A", live: true, start: 0 }]);
    h.unmount();
  });

  it("commits the latest worker projection and resets on non-prefix text", () => {
    const h = renderHook(() => useStreamingMarkdownBlocks("A", true));
    const worker = FakeWorker.instances[0]!;
    respond(worker, {
      id: 1,
      blocks: [{ content: "changed", live: true, start: 0 }],
      text: "changed",
    });
    // "A" does not start with the worker's "changed" text → reset to a single block.
    expect(h.current).toEqual([{ content: "A", live: true, start: 0 }]);
    h.unmount();
  });

  it("extends the tail block for prefix-extension text", () => {
    let text = "A";
    const h = renderHook(() => useStreamingMarkdownBlocks(text, true));
    const worker = FakeWorker.instances[0]!;
    respond(worker, {
      id: 1,
      blocks: [{ content: "A", live: true, start: 0 }],
      text: "A",
    });
    text = "AB";
    h.rerender();
    expect(h.current).toEqual([{ content: "AB", live: true, start: 0 }]);
    h.unmount();
  });

  it("queues a superseded request while one is in flight", () => {
    let text = "A";
    const h = renderHook(() => useStreamingMarkdownBlocks(text, true));
    const worker = FakeWorker.instances[0]!;
    text = "AB";
    h.rerender();
    // The in-flight request has not answered yet, so the newer one is queued.
    expect(worker.posted).toEqual([{ id: 1, live: true, text: "A" }]);
    respond(worker, {
      id: 1,
      blocks: [{ content: "A", live: true, start: 0 }],
      text: "A",
    });
    expect(worker.posted).toEqual([
      { id: 1, live: true, text: "A" },
      { id: 2, live: true, text: "AB" },
    ]);
    h.unmount();
  });

  it("terminates the worker when a completed segment is no longer live", () => {
    let live = true;
    const h = renderHook(() => useStreamingMarkdownBlocks("A", live));
    const worker = FakeWorker.instances[0]!;
    respond(worker, {
      id: 1,
      blocks: [{ content: "A", live: true, start: 0 }],
      text: "A",
    });
    expect(worker.terminated).toBe(false);
    live = false;
    h.rerender();
    respond(worker, {
      id: 2,
      blocks: [{ content: "A", live: false, start: 0 }],
      text: "A",
    });
    expect(worker.terminated).toBe(true);
    h.unmount();
  });

  it("falls back to synchronous projection after a worker error", () => {
    let text = "A";
    const h = renderHook(() => useStreamingMarkdownBlocks(text, true));
    const worker = FakeWorker.instances[0]!;
    fail(worker);
    expect(worker.terminated).toBe(true);
    text = "AB";
    h.rerender();
    expect(h.current).toEqual([{ content: "AB", live: true, start: 0 }]);
    h.unmount();
  });
});
