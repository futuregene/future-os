import type { StreamingMarkdownWorkerRequest, StreamingMarkdownWorkerResponse } from "./streamingMarkdown.worker";
import type { StreamingMarkdownBlock } from "./streamingMarkdownBlocks";
import { useEffect, useMemo, useRef, useState } from "react";
import { plainStreamingMarkdown, projectStreamingMarkdown } from "./streamingMarkdownBlocks";

const MAX_WORKER_FAILURES = 3;
// A dead worker must not turn a large reply into an unbounded UI-thread parse.
const SYNC_FALLBACK_MAX_CHARS = 8192;

function fallbackBlocks(text: string, live: boolean): StreamingMarkdownBlock[] {
  if (text.length <= SYNC_FALLBACK_MAX_CHARS) {
    try {
      return projectStreamingMarkdown(text, live);
    }
    catch {
      // Preserve the source as text if parsing itself fails.
    }
  }
  return plainStreamingMarkdown(text, live);
}

interface Projection {
  blocks: StreamingMarkdownBlock[];
  text: string;
}

function provisionalProjection(current: Projection, text: string, live: boolean): StreamingMarkdownBlock[] {
  if (!text.startsWith(current.text) || current.blocks.length === 0)
    return plainStreamingMarkdown(text, live);
  const blocks = current.blocks.slice();
  const tail = blocks[blocks.length - 1]!;
  const suffix = text.slice(current.text.length);
  if (!suffix && tail.live === live)
    return current.blocks;
  // Display every incoming character immediately without reparsing the growing
  // table/list/paragraph on the UI thread. The next worker result incorporates
  // this literal suffix into its proper Markdown structure.
  blocks[blocks.length - 1] = {
    ...tail,
    content: tail.content + suffix,
    live,
    document: suffix && tail.document
      ? {
          ...tail.document,
          raw: tail.content + suffix,
          nodes: [...tail.document.nodes, { type: "paragraph", children: [{ type: "text", text: suffix }] }],
        }
      : tail.document,
  };
  return blocks;
}

/** One parse in flight plus one latest request; never parse a live tail in render. */
export function useStreamingMarkdownBlocks(text: string, live: boolean): StreamingMarkdownBlock[] {
  const streamedRef = useRef(live);
  streamedRef.current ||= live;
  const shouldProject = streamedRef.current;
  const [projection, setProjection] = useState<Projection>(() => ({
    blocks: shouldProject
      ? typeof Worker === "undefined" ? fallbackBlocks(text, live) : plainStreamingMarkdown(text, live)
      : [],
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
    if (!shouldProject)
      return;
    if (typeof Worker === "undefined" || workerFailuresRef.current >= MAX_WORKER_FAILURES) {
      setProjection({ blocks: fallbackBlocks(text, live), text });
      return;
    }

    const fail = () => {
      workerFailuresRef.current += 1;
      activeRef.current = false;
      queuedRef.current = null;
      workerRef.current?.terminate();
      workerRef.current = null;
      const currentText = textRef.current;
      setProjection({ blocks: fallbackBlocks(currentText, liveRef.current), text: currentText });
    };
    const post = (worker: Worker, request: StreamingMarkdownWorkerRequest) => {
      activeRef.current = true;
      try {
        worker.postMessage(request);
      }
      catch {
        fail();
      }
    };

    let worker = workerRef.current;
    if (!worker) {
      try {
        worker = new Worker(new URL("./streamingMarkdown.worker.ts", import.meta.url), { type: "module" });
      }
      catch {
        fail();
        return;
      }
      workerRef.current = worker;
      const created = worker;
      worker.onmessage = (event: MessageEvent<StreamingMarkdownWorkerResponse>) => {
        if (workerRef.current !== created)
          return;
        activeRef.current = false;
        workerFailuresRef.current = 0;
        const latest = event.data.id === latestIdRef.current;
        // An older compatible prefix still advances block boundaries; rejecting
        // all non-latest responses would starve a parser slower than the stream.
        if (textRef.current.startsWith(event.data.text)) {
          setProjection((previous) => {
            const byStart = new Map(previous.blocks.map(block => [block.start, block]));
            return {
              text: event.data.text,
              blocks: event.data.blocks.map((block) => {
                const old = byStart.get(block.start);
                // Structured cloning creates fresh ASTs for unchanged blocks.
                // Preserve their object identity so memoized renderers can skip.
                return old?.parsed && old.content === block.content && old.live === block.live
                  ? old
                  : block;
              }),
            };
          });
        }
        const queued = queuedRef.current;
        queuedRef.current = null;
        if (queued) {
          post(created, queued);
        }
        else if (latest && !liveRef.current) {
          created.terminate();
          workerRef.current = null;
        }
      };
      const handleFailure = () => {
        if (workerRef.current === created)
          fail();
      };
      worker.onerror = handleFailure;
      worker.onmessageerror = handleFailure;
    }

    const request = { id: ++latestIdRef.current, live, text };
    if (activeRef.current)
      queuedRef.current = request;
    else
      post(worker, request);
  }, [live, shouldProject, text]);

  useEffect(() => () => {
    workerRef.current?.terminate();
    workerRef.current = null;
    activeRef.current = false;
    queuedRef.current = null;
  }, []);

  return useMemo(() => {
    if (!shouldProject)
      return [{ content: text, live: false, start: 0 }];
    return provisionalProjection(projection, text, live);
  }, [live, projection, shouldProject, text]);
}
