import { useSyncExternalStore } from "react";
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
        sessionName: typeof raw.sessionName === "string" ? raw.sessionName : null,
        cwd: typeof raw.cwd === "string" ? raw.cwd : null,
        parentSessionId: typeof raw.parentSessionId === "string" ? raw.parentSessionId : null,
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
  void getAgentState(threadId);
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
