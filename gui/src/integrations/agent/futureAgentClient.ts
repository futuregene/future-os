import { invoke } from "@tauri-apps/api/core";

interface AgentPromptResponse {
  content: string;
}

export async function sendPromptToFutureAgent(
  message: string,
  threadId: string,
  sessionId?: string | null,
  runId?: string | null,
  modelId?: string | null,
  imagePaths?: string[],
  thinkingLevel?: string | null,
) {
  const response = await invoke<AgentPromptResponse>("agent_prompt", {
    imagePaths: imagePaths ?? [],
    message,
    sessionId: sessionId ?? null,
    threadId,
    runId: runId ?? null,
    modelId: modelId ?? null,
    thinkingLevel: thinkingLevel ?? null,
  });
  return response.content;
}
