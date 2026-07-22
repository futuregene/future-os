import { useSyncExternalStore } from "react";
import { invokeCommand } from "../tauri/invoke";

/** Agent-side session state, fetched via get_state RPC. */
export interface AgentSessionState {
  model?: string | null;
  thinkingLevel?: string | null;
  sessionName?: string | null;
  cwd?: string | null;
  parentSessionId?: string | null;
  /** Whether the agent is currently streaming a response for this session. */
  isStreaming?: boolean;
}

interface CacheEntry {
  state: AgentSessionState;
  /** Timestamp when this entry was fetched (Date.now() ms). */
  fetchedAt: number;
}

/** TTL for cached agent state (30s). Agent restarts invalidate the cache. */
const CACHE_TTL_MS = 30_000;
const CACHE_MAX = 100;

const cache = new Map<string, CacheEntry>();
const inFlight = new Map<string, Promise<AgentSessionState>>();
// Incremented by optimistic updates/invalidation. A request may populate the
// cache only if no newer local mutation happened while it was in flight.
const versions = new Map<string, number>();

// Subscribers (React components via useCachedAgentState) notified on every cache
// mutation, so a background fetch updates the UI immediately instead of waiting
// for an unrelated re-render (e.g. the 1.5s run-status poll tick).
const listeners = new Set<() => void>();

function notify() {
  for (const listener of listeners)
    listener();
}

// Module-scoped so the reference stays stable across renders — otherwise
// useSyncExternalStore re-subscribes on every render.
function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

/** Fetch session state from the agent, caching the result for CACHE_TTL_MS. */
export async function getAgentState(threadId: string): Promise<AgentSessionState> {
  const now = Date.now();
  const cached = cache.get(threadId);
  if (cached && now - cached.fetchedAt < CACHE_TTL_MS) {
    touchCache(threadId, cached);
    return cached.state;
  }

  const pending = inFlight.get(threadId);
  if (pending)
    return pending;

  const requestVersion = versions.get(threadId) ?? 0;
  const request = invokeCommand<Record<string, unknown>>("get_thread_agent_state", { threadId })
    .then((raw) => {
      const state: AgentSessionState = {
        model: typeof raw.model === "string" ? raw.model : null,
        thinkingLevel: typeof raw.thinkingLevel === "string" ? raw.thinkingLevel : null,
        sessionName: typeof raw.session_name === "string" ? raw.session_name : null,
        cwd: typeof raw.cwd === "string" ? raw.cwd : null,
        parentSessionId: typeof raw.parentSessionId === "string" ? raw.parentSessionId : null,
        isStreaming: typeof raw.isStreaming === "boolean" ? raw.isStreaming : undefined,
      };
      if ((versions.get(threadId) ?? 0) === requestVersion) {
        cache.set(threadId, { state, fetchedAt: Date.now() });
        pruneCache();
        notify();
        return state;
      }
      return cache.get(threadId)?.state ?? state;
    })
    .finally(() => {
      if (inFlight.get(threadId) === request)
        inFlight.delete(threadId);
    });
  inFlight.set(threadId, request);
  return request;
}

/**
 * Update cached state after a model/thinking change (optimistic). Replaces the
 * state object (rather than mutating in place) so useSyncExternalStore's
 * Object.is snapshot comparison detects the change and re-renders subscribers.
 */
export function updateCachedAgentState(threadId: string, patch: Partial<AgentSessionState>) {
  versions.set(threadId, (versions.get(threadId) ?? 0) + 1);
  inFlight.delete(threadId);
  const cached = cache.get(threadId);
  cache.set(threadId, {
    state: cached ? { ...cached.state, ...patch } : (patch as AgentSessionState),
    // Always use the current time — an optimistic update from the user's
    // explicit action must not inherit a stale fetchedAt that would make
    // getCachedAgentState treat it as expired immediately.
    fetchedAt: Date.now(),
  });
  pruneCache();
  notify();
}

/** Synchronously read cached state (no fetch). Returns undefined on miss. */
export function getCachedAgentState(threadId: string | undefined | null): AgentSessionState | undefined {
  if (!threadId)
    return undefined;
  const cached = cache.get(threadId);
  if (cached && Date.now() - cached.fetchedAt < CACHE_TTL_MS) {
    return cached.state;
  }
  return undefined;
}

/** Invalidate a thread's cached state (force re-fetch on next access). */
export function invalidateAgentState(threadId: string) {
  versions.set(threadId, (versions.get(threadId) ?? 0) + 1);
  inFlight.delete(threadId);
  if (cache.delete(threadId))
    notify();
}

function touchCache(threadId: string, entry: CacheEntry) {
  cache.delete(threadId);
  cache.set(threadId, entry);
}

function pruneCache() {
  while (cache.size > CACHE_MAX) {
    const oldest = cache.keys().next().value;
    if (!oldest)
      return;
    cache.delete(oldest);
    versions.delete(oldest);
    inFlight.delete(oldest);
  }
}

/** Pre-fetch agent state for a thread in the background. */
export function prefetchAgentState(threadId: string | undefined | null) {
  if (!threadId)
    return;
  // Fire-and-forget: the agent may be offline or the thread may have no session
  // yet, so swallow the rejection here — awaiting callers still see it.
  void getAgentState(threadId).catch(() => {});
}

/**
 * Reactive read of cached agent state: subscribes to cache mutations so a
 * background fetch (prefetchAgentState) or optimistic update re-renders the
 * caller as soon as the value lands, without waiting for an unrelated tick.
 * Returns the same object reference until the entry changes, keeping
 * useSyncExternalStore's snapshot stable.
 */
export function useCachedAgentState(threadId: string | undefined | null): AgentSessionState | undefined {
  return useSyncExternalStore(subscribe, () => getCachedAgentState(threadId));
}

// ── Streaming-status cache (short TTL, separate from full agent state) ────

const STREAMING_TTL_MS = 2_000;
const streamingCache = new Map<string, { streaming: boolean; fetchedAt: number }>();

/**
 * Lightweight check: is this session currently streaming?
 * Uses a short-lived cache (2s) so the thread list picks up TUI-initiated
 * streaming quickly without hammering the agent with get_state RPCs.
 */
export async function fetchSessionStreaming(threadId: string): Promise<boolean> {
  const cached = streamingCache.get(threadId);
  if (cached && Date.now() - cached.fetchedAt < STREAMING_TTL_MS) {
    return cached.streaming;
  }
  try {
    const raw = await invokeCommand<Record<string, unknown>>("get_thread_agent_state", { threadId });
    const streaming = raw.isStreaming === true;
    streamingCache.set(threadId, { streaming, fetchedAt: Date.now() });
    // Also update the full cache so useCachedAgentState stays in sync
    if (streaming) {
      invalidateAgentState(threadId);
    }
    return streaming;
  } catch {
    return cached?.streaming ?? false;
  }
}

/**
 * Poll streaming status for a batch of threads and return a map of
 * threadId → isStreaming. Fires all requests in parallel.
 */
export async function pollStreamingStatuses(
  threadIds: string[],
): Promise<Record<string, boolean>> {
  const entries = await Promise.all(
    threadIds.map(async (id) => {
      try {
        const streaming = await fetchSessionStreaming(id);
        return { id, streaming };
      } catch {
        return { id, streaming: false };
      }
    }),
  );
  const result: Record<string, boolean> = {};
  for (const entry of entries) {
    result[entry.id] = entry.streaming;
  }
  return result;
}
