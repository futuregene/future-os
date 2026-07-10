import { invokeCommand } from "../tauri/invoke";

/** Agent-side session state, fetched via get_state RPC. */
export interface AgentSessionState {
  model?: string | null;
  thinkingLevel?: string | null;
  sessionName?: string | null;
  cwd?: string | null;
  parentSessionId?: string | null;
}

interface CacheEntry {
  state: AgentSessionState;
  /** Timestamp when this entry was fetched (Date.now() ms). */
  fetchedAt: number;
}

/** TTL for cached agent state (30s). Agent restarts invalidate the cache. */
const CACHE_TTL_MS = 30_000;

const cache = new Map<string, CacheEntry>();

/** Fetch session state from the agent, caching the result for CACHE_TTL_MS. */
export async function getAgentState(threadId: string): Promise<AgentSessionState> {
  const now = Date.now();
  const cached = cache.get(threadId);
  if (cached && now - cached.fetchedAt < CACHE_TTL_MS) {
    return cached.state;
  }

  const raw = await invokeCommand<Record<string, unknown>>("get_thread_agent_state", { threadId });
  const state: AgentSessionState = {
    model: typeof raw.model === "string" ? raw.model : null,
    thinkingLevel: typeof raw.thinkingLevel === "string" ? raw.thinkingLevel : null,
    sessionName: typeof raw.sessionName === "string" ? raw.sessionName : null,
    cwd: typeof raw.cwd === "string" ? raw.cwd : null,
    parentSessionId: typeof raw.parentSessionId === "string" ? raw.parentSessionId : null,
  };

  cache.set(threadId, { state, fetchedAt: now });
  return state;
}

/** Update cached state after a model/thinking change (optimistic). */
export function updateCachedAgentState(threadId: string, patch: Partial<AgentSessionState>) {
  const cached = cache.get(threadId);
  if (cached) {
    Object.assign(cached.state, patch);
  } else {
    cache.set(threadId, { state: patch as AgentSessionState, fetchedAt: Date.now() });
  }
}

/** Synchronously read cached state (no fetch). Returns undefined on miss. */
export function getCachedAgentState(threadId: string | undefined | null): AgentSessionState | undefined {
  if (!threadId) return undefined;
  const cached = cache.get(threadId);
  if (cached && Date.now() - cached.fetchedAt < CACHE_TTL_MS) {
    return cached.state;
  }
  return undefined;
}

/** Invalidate a thread's cached state (force re-fetch on next access). */
export function invalidateAgentState(threadId: string) {
  cache.delete(threadId);
}

/** Pre-fetch agent state for a thread in the background. */
export function prefetchAgentState(threadId: string | undefined | null) {
  if (!threadId) return;
  void getAgentState(threadId);
}
