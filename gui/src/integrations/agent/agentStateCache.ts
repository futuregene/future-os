import { listen } from "@tauri-apps/api/event";
import { useSyncExternalStore } from "react";

// ── Real-time agent state updates via Tauri events ──────────────────────

import { invokeCommand } from "../tauri/invoke";

/** Agent-side session state, fetched via get_state RPC. */
export interface AgentSessionState {
  model?: string | null;
  thinkingLevel?: string | null;
  sessionName?: string | null;
  sessionId?: string | null;
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
        sessionId: typeof raw.sessionId === "string" ? raw.sessionId : null,
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

let eventListenerInstalled = false;

/**
 * Install a one-time Tauri event listener that processes ALL agent events
 * (user_message, text_chunk, settings changes, etc.) forwarded from the
 * StreamEvents observer.  This gives the GUI the same real-time latency
 * as the TUI — no polling, no synthetic run delay.
 */
export function installAgentEventListener() {
  if (eventListenerInstalled)
    return;
  eventListenerInstalled = true;

  void listen<Record<string, unknown>>("agent-event", (event) => {
    const p = event.payload;
    if (!p)
      return;

    const sessionId = typeof p.sessionId === "string" ? p.sessionId : null;
    const eventType = typeof p._eventType === "string" ? p._eventType : null;
    if (!sessionId || !eventType)
      return;

    switch (eventType) {
      // ── Settings-change events: update cache ──
      case "model_changed":
      case "thinking_level_changed":
      case "permission_level_changed":
      case "session_name_changed":
      case "cwd_changed":
      case "config_reloaded":
        applySettingsEvent(sessionId, eventType, p);
        break;

      // ── Content events: forward to the active AgentThread via a
      //     window custom event so the message list updates in real-time.
      case "user_message":
      case "text_chunk":
      case "agent_start":
      case "agent_end":
      case "thinking_start":
      case "thinking_delta":
      case "thinking_end":
      case "tool_start":
      case "tool_delta":
      case "tool_end":
        window.dispatchEvent(new CustomEvent("future:agent-event", {
          detail: { sessionId, eventType, payload: p },
        }));
        break;
    }
  });
}

/** Apply a settings-change event to the agent state cache. */
function applySettingsEvent(
  sessionId: string,
  eventType: string,
  p: Record<string, unknown>,
) {
  // cwd_changed must reconcile workspace even when the session isn't yet
  // in the agent-state cache (e.g. TUI /cwd on a just-imported session
  // whose state hasn't been fetched). Fire it unconditionally — once per
  // event, regardless of how many cached threads share the session.
  if (eventType === "cwd_changed" && typeof p.cwd === "string") {
    invokeCommand("reconcile_thread_workspace", {
      sessionId,
      cwd: p.cwd,
    }).then(() => {
      window.dispatchEvent(new CustomEvent("future:cwd-changed"));
    }).catch(() => {});
  }

  for (const [threadId, entry] of cache) {
    if (entry.state.sessionId !== sessionId)
      continue;

    const next = { ...entry.state };
    let changed = false;

    switch (eventType) {
      case "model_changed":
        if (typeof p.model === "string") {
          next.model = p.model;
          changed = true;
        }
        break;
      case "thinking_level_changed":
        if (typeof p.level === "string") {
          next.thinkingLevel = p.level;
          changed = true;
        }
        break;
      case "session_name_changed":
        if (typeof p.name === "string") {
          next.sessionName = p.name;
          changed = true;
        }
        break;
      case "cwd_changed":
        if (typeof p.cwd === "string") {
          next.cwd = p.cwd;
          changed = true;
          // reconcile_thread_workspace already called above
        }
        break;
      case "config_reloaded":
        versions.set(threadId, (versions.get(threadId) ?? 0) + 1);
        cache.delete(threadId);
        changed = true;
        break;
    }

    if (changed) {
      cache.set(threadId, { state: next, fetchedAt: Date.now() });
    }
    // Don't break — multiple threads can share the same agent session.
  }
  notify();
}

// ── Bulk streaming-status poll (no per-thread get_state fan-out) ────────

/**
 * Bulk streaming-status poll: ONE Tauri command returns every streaming
 * thread id. The agent only scans its in-memory session map (no hydration,
 * no disk I/O), so polling never creates agent sessions/loops for threads
 * the user hasn't opened — unlike the old per-thread get_state fan-out,
 * which hydrated every polled session at startup.
 */
export async function pollStreamingThreadIds(): Promise<string[]> {
  try {
    const raw = await invokeCommand<string[]>("list_streaming_thread_ids");
    return Array.isArray(raw) ? raw : [];
  }
  catch {
    // Agent unreachable: report "nothing streaming" — the next tick retries.
    return [];
  }
}
