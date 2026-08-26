import type { StreamingMarkdownBlock } from "@future-os/markdown";
import type { StreamingMarkdownWorkerRequest, StreamingMarkdownWorkerResponse } from "./streamingMarkdown.worker";
import { splitStreamingMarkdown } from "@future-os/markdown";
import { useEffect, useMemo, useRef, useState } from "react";

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
    blocks: shouldProject ? splitStreamingMarkdown(text, live) : [],
    text,
  }));
  const workerRef = useRef<Worker | null>(null);
  const activeRef = useRef(false);
  const queuedRef = useRef<StreamingMarkdownWorkerRequest | null>(null);
  const latestIdRef = useRef(0);
  const liveRef = useRef(live);
  const textRef = useRef(text);
  const workerFailedRef = useRef(false);
  liveRef.current = live;
  textRef.current = text;

  useEffect(() => {
    if (!shouldProject || typeof Worker === "undefined" || workerFailedRef.current) {
      if (shouldProject)
        setProjection({ blocks: splitStreamingMarkdown(text, live), text });
      return;
    }

    let worker = workerRef.current;
    if (!worker) {
      worker = new Worker(new URL("./streamingMarkdown.worker.ts", import.meta.url), { type: "module" });
      workerRef.current = worker;
      worker.onmessage = (event: MessageEvent<StreamingMarkdownWorkerResponse>) => {
        activeRef.current = false;
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
        workerFailedRef.current = true;
        activeRef.current = false;
        queuedRef.current = null;
        workerRef.current?.terminate();
        workerRef.current = null;
        const currentText = textRef.current;
        setProjection({
          blocks: splitStreamingMarkdown(currentText, liveRef.current),
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
