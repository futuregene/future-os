import type { StreamingMarkdownBlock } from "./streamingMarkdownBlocks";
import { createStreamingMarkdownProjector } from "./streamingMarkdownBlocks";

export interface StreamingMarkdownWorkerRequest {
  id: number;
  live: boolean;
  text: string;
}

export interface StreamingMarkdownWorkerResponse {
  blocks: StreamingMarkdownBlock[];
  id: number;
  text: string;
}

const workerScope = globalThis as unknown as {
  onmessage: ((event: MessageEvent<StreamingMarkdownWorkerRequest>) => void) | null;
  postMessage: (message: StreamingMarkdownWorkerResponse) => void;
};

const project = createStreamingMarkdownProjector();

workerScope.onmessage = (event: MessageEvent<StreamingMarkdownWorkerRequest>) => {
  const { id, live, text } = event.data;
  const response: StreamingMarkdownWorkerResponse = {
    blocks: project(text, live),
    id,
    text,
  };
  workerScope.postMessage(response);
};
