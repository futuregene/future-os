import type { StreamingMarkdownWorkerRequest, StreamingMarkdownWorkerResponse } from "./streamingMarkdown.worker";
import type { StreamingMarkdownBlock } from "./streamingMarkdownBlocks";
import { useEffect, useMemo, useRef, useState } from "react";
import { splitStreamingMarkdown } from "./streamingMarkdownBlocks";

/**
 * Consecutive off-thread parse failures tolerated before this component gives up
 * on the worker and parses synchronously. The counter resets on every successful
 * response, so an intermittent failure (memory pressure, a worker killed under a
 * multi-hundred-KB block) keeps using the off-thread parser.
 *
 * Latching on the FIRST failure is what makes a transient hiccup permanent: the
 * synchronous fallback re-parses the whole accumulated block on the UI thread,
 * which for a high-throughput reasoning model is ~100ms per push at up to 60
 * pushes/s — the chat stops responding while the Agent keeps streaming fine.
 */
const MAX_WORKER_FAILURES = 3;

/**
 * Never let a parser throw escape into the render tree. Pathologically nested
 * markdown (a long reasoning reply is full of nested lists/quotes) can exhaust
 * the parser's stack; degrading to one undivided block shows slightly worse
 * placement instead of unmounting the conversation.
 */
function safeSplitStreamingMarkdown(text: string, live: boolean): StreamingMarkdownBlock[] {
  try {
    return splitStreamingMarkdown(text, live);
  }
  catch {
    return [{ content: text, live, start: 0 }];
  }
}

interface Projection {
  blocks: StreamingMarkdownBlock[];
  text: string;
}

function provisionalProjection(current: Projection, text: string, live: boolean): StreamingMarkdownBlock[] {
  if (current.text === text)
    return current.blocks;
  if (!text.startsWith(current.text) || current.blocks.length === 0)
    return [{ content: text, live, start: 0 }];

  const suffix = text.slice(current.text.length);
  const blocks = current.blocks.slice();
  const tail = blocks[blocks.length - 1]!;
  blocks[blocks.length - 1] = {
    ...tail,
    content: tail.content + suffix,
    live,
  };
  return blocks;
}

/**
 * Parse streaming block boundaries off the UI thread. At most one request runs
 * and one latest request waits; intermediate projections are superseded.
 */
export function useStreamingMarkdownBlocks(text: string, live: boolean): StreamingMarkdownBlock[] {
  const streamedRef = useRef(live);
  streamedRef.current ||= live;
  const shouldProject = streamedRef.current;
  const [projection, setProjection] = useState<Projection>(() => ({
    blocks: shouldProject ? safeSplitStreamingMarkdown(text, live) : [],
    text,
  }));
  const workerRef = useRef<Worker | null>(null);
  const activeRef = useRef(false);
  const queuedRef = useRef<StreamingMarkdownWorkerRequest | null>(null);
  const latestIdRef = useRef(0);
  const liveRef = useRef(live);
  const textRef = useRef(text);
  const workerFailuresRef = useRef(0);
  liveRef.current = live;
  textRef.current = text;

  useEffect(() => {
    if (!shouldProject || typeof Worker === "undefined" || workerFailuresRef.current >= MAX_WORKER_FAILURES) {
      if (shouldProject)
        setProjection({ blocks: safeSplitStreamingMarkdown(text, live), text });
      return;
    }

    let worker = workerRef.current;
    if (!worker) {
      worker = new Worker(new URL("./streamingMarkdown.worker.ts", import.meta.url), { type: "module" });
      workerRef.current = worker;
      worker.onmessage = (event: MessageEvent<StreamingMarkdownWorkerResponse>) => {
        activeRef.current = false;
        // A response proves the off-thread parser works, so earlier transient
        // failures must not count toward the give-up budget.
        workerFailuresRef.current = 0;
        const latest = event.data.id === latestIdRef.current;
        if (latest) {
          setProjection({ blocks: event.data.blocks, text: event.data.text });
        }
        const queued = queuedRef.current;
        queuedRef.current = null;
        if (queued) {
          activeRef.current = true;
          workerRef.current?.postMessage(queued);
        }
        else if (latest && !liveRef.current) {
          // A completed segment keeps its final block projection in React state;
          // it no longer needs a dedicated worker for the rest of the thread's
          // lifetime. A later transition back to live lazily creates a new one.
          workerRef.current?.terminate();
          workerRef.current = null;
        }
      };
      worker.onerror = () => {
        workerFailuresRef.current += 1;
        activeRef.current = false;
        queuedRef.current = null;
        workerRef.current?.terminate();
        workerRef.current = null;
        const currentText = textRef.current;
        setProjection({
          blocks: safeSplitStreamingMarkdown(currentText, liveRef.current),
          text: currentText,
        });
      };
    }

    const request: StreamingMarkdownWorkerRequest = {
      id: ++latestIdRef.current,
      live,
      text,
    };
    if (activeRef.current) {
      queuedRef.current = request;
    }
    else {
      activeRef.current = true;
      worker.postMessage(request);
    }
  }, [live, shouldProject, text]);

  useEffect(() => () => {
    workerRef.current?.terminate();
    workerRef.current = null;
    queuedRef.current = null;
  }, []);

  return useMemo(() => {
    if (!shouldProject)
      return [{ content: text, live: false, start: 0 }];
    return provisionalProjection(projection, text, live);
  }, [live, projection, shouldProject, text]);
}
