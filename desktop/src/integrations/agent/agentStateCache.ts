import { listen } from "@tauri-apps/api/event";
import { useSyncExternalStore } from "react";

// ── Real-time agent state updates via Tauri events ──────────────────────

import i18n from "../../i18n";
import { emitFutureEvent } from "../../lib/futureEvents";
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
  activeRun?: {
    runId: string;
    epoch: number;
    state:
      | "starting"
      | "running"
      | "cancelling"
      | "cancellation_stuck"
      | "finalizing";
    lastEventIdx: number;
  } | null;
}

interface CacheEntry {
  state: AgentSessionState;
  /** Timestamp when this entry was fetched (Date.now() ms). */
  fetchedAt: number;
}

/**
 * Revalidation throttle for getAgentState (30s). Freshness between fetches
 * comes from the agent's push events (applySettingsEvent) and optimistic
 * updates; this gate only limits how often an explicit fetch (thread
 * activation, post-rename) actually hits the agent. Synchronous reads are
 * stale-while-revalidate regardless of it.
 */
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
  for (const listener of listeners) listener();
}

// Module-scoped so the reference stays stable across renders — otherwise
// useSyncExternalStore re-subscribes on every render.
function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

/**
 * Fetch session state from the agent. A fresh entry is returned as-is; an
 * older one is revalidated at most once per CACHE_TTL_MS (in-flight requests
 * are deduped either way). With `force`, the TTL throttle is bypassed — the
 * stale snapshot stays readable to synchronous callers while the fresh fetch
 * lands.
 */
export async function getAgentState(
  threadId: string,
  options?: { force?: boolean },
): Promise<AgentSessionState> {
  const now = Date.now();
  const cached = cache.get(threadId);
  if (!options?.force && cached && now - cached.fetchedAt < CACHE_TTL_MS) {
    touchCache(threadId, cached);
    return cached.state;
  }

  const pending = inFlight.get(threadId);
  if (pending)
    return pending;

  const requestVersion = versions.get(threadId) ?? 0;
  const request = invokeCommand<Record<string, unknown>>(
    "get_thread_agent_state",
    { threadId },
  )
    .then((raw) => {
      const state: AgentSessionState = {
        model: typeof raw.model === "string" ? raw.model : null,
        thinkingLevel:
          typeof raw.thinkingLevel === "string" ? raw.thinkingLevel : null,
        sessionName:
          typeof raw.sessionName === "string"
            ? raw.sessionName
            : typeof raw.session_name === "string"
              ? raw.session_name
              : null,
        sessionId: typeof raw.sessionId === "string" ? raw.sessionId : null,
        cwd: typeof raw.cwd === "string" ? raw.cwd : null,
        parentSessionId:
          typeof raw.parentSessionId === "string" ? raw.parentSessionId : null,
        isStreaming:
          typeof raw.isStreaming === "boolean" ? raw.isStreaming : undefined,
        activeRun: parseActiveRun(raw.activeRun),
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

function parseActiveRun(value: unknown): AgentSessionState["activeRun"] {
  if (!value || typeof value !== "object")
    return null;
  const run = value as Record<string, unknown>;
  if (typeof run.runId !== "string" || typeof run.state !== "string")
    return null;
  return {
    runId: run.runId,
    epoch: typeof run.epoch === "number" ? run.epoch : 0,
    state: run.state as NonNullable<AgentSessionState["activeRun"]>["state"],
    lastEventIdx: typeof run.lastEventIdx === "number" ? run.lastEventIdx : -1,
  };
}

/**
 * Update cached state after a model/thinking change (optimistic). Replaces the
 * state object (rather than mutating in place) so useSyncExternalStore's
 * Object.is snapshot comparison detects the change and re-renders subscribers.
 */
export function updateCachedAgentState(
  threadId: string,
  patch: Partial<AgentSessionState>,
) {
  versions.set(threadId, (versions.get(threadId) ?? 0) + 1);
  inFlight.delete(threadId);
  const cached = cache.get(threadId);
  cache.set(threadId, {
    state: cached
      ? { ...cached.state, ...patch }
      : (patch as AgentSessionState),
    // Always use the current time — an optimistic update from the user's
    // explicit action must not inherit a stale fetchedAt that would make
    // getCachedAgentState treat it as expired immediately.
    fetchedAt: Date.now(),
  });
  pruneCache();
  notify();
}

/**
 * Synchronously read cached state (no fetch). Returns undefined only when the
 * thread was never fetched. Stale entries ARE returned (stale-while-revalidate):
 * silently dropping the snapshot at the TTL boundary made the composer fall
 * back to the global draft model/thinking level ~30s into viewing a thread.
 * Freshness is the writer's job — getAgentState still refetches once an entry
 * is older than CACHE_TTL_MS.
 */
export function getCachedAgentState(
  threadId: string | undefined | null,
): AgentSessionState | undefined {
  if (!threadId)
    return undefined;
  return cache.get(threadId)?.state;
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
    // size > CACHE_MAX (>= 1) guarantees a first key exists.
    const oldest = cache.keys().next().value!;
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
 * Revalidate a thread's agent state regardless of the TTL throttle. Used when
 * the world changed out from under the cache — an agent restart/reconnect,
 * which can happen well inside the TTL window; a plain prefetch would
 * short-circuit on the fresh-looking entry and nothing else would trigger a
 * fetch while the user stays on the thread. The stale snapshot remains
 * available to synchronous readers until the fresh one lands.
 */
export function revalidateAgentState(threadId: string | undefined | null) {
  if (!threadId)
    return;
  void getAgentState(threadId, { force: true }).catch(() => {});
}

/**
 * Reactive read of cached agent state: subscribes to cache mutations so a
 * background fetch (prefetchAgentState) or optimistic update re-renders the
 * caller as soon as the value lands, without waiting for an unrelated tick.
 * Returns the same object reference until the entry changes, keeping
 * useSyncExternalStore's snapshot stable.
 */
export function useCachedAgentState(
  threadId: string | undefined | null,
): AgentSessionState | undefined {
  return useSyncExternalStore(subscribe, () => getCachedAgentState(threadId));
}

let eventListenerInstalled = false;

/**
 * Install a one-time Tauri event listener that processes ALL agent events
 * (user messages, compaction lifecycle, settings changes, etc.) forwarded
 * from the StreamEvents observer. This gives the GUI the same real-time
 * latency as the TUI — no polling, no synthetic run delay.
 */
export function installAgentEventListener() {
  if (eventListenerInstalled)
    return;
  eventListenerInstalled = true;

  void listen<Record<string, unknown>>("provider-config-changed", (event) => {
    const p = event.payload ?? {};
    emitFutureEvent("providers-changed", {
      revision: typeof p.revision === "number" ? p.revision : 0,
      providerId: typeof p.providerId === "string" ? p.providerId : "*",
      operation: typeof p.operation === "string" ? p.operation : "snapshot",
      authChanged: p.authChanged !== false,
      modelsChanged: p.modelsChanged !== false,
    });
  });

  void listen<Record<string, unknown>>("agent-event", (event) => {
    const p = event.payload;
    if (!p)
      return;

    const threadId = typeof p.threadId === "string" ? p.threadId : null;
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
      //     Only the types the consumer handles (useThreadMessages) are
      //     forwarded — dispatching every text/thinking/tool delta fired a
      //     window CustomEvent per token with no listener doing anything
      //     with it; those deltas reach the view through the run-event
      //     projection path instead.
      case "user_message":
      case "agent_start":
      case "agent_end":
      case "compaction_started":
      case "compaction_committed":
      case "compaction_failed":
        if (!threadId)
          return;
        window.dispatchEvent(
          new CustomEvent("future:agent-event", {
            detail: { threadId, sessionId, eventType, payload: p },
          }),
        );
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
    })
      .then(() => {
        window.dispatchEvent(new CustomEvent("future:cwd-changed"));
      })
      .catch((error: unknown) => {
        emitFutureEvent("toast", {
          message: i18n.t("agent:thread.workspaceAccessError", {
            message: String(error),
          }),
          tone: "error",
        });
      });
  }

  const revalidate: string[] = [];
  for (const [threadId, entry] of cache) {
    if (entry.state.sessionId !== sessionId)
      continue;

    if (eventType === "config_reloaded") {
      // The agent rebuilt this session's settings: drop the snapshot and
      // revalidate it right away. Re-inserting the pre-reload state with a
      // fresh fetchedAt here used to defeat the delete and keep a stale
      // model/thinking level alive indefinitely.
      versions.set(threadId, (versions.get(threadId) ?? 0) + 1);
      cache.delete(threadId);
      revalidate.push(threadId);
      continue;
    }

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
    }

    if (changed) {
      cache.set(threadId, { state: next, fetchedAt: Date.now() });
    }
    // Don't break — multiple threads can share the same agent session.
  }
  notify();
  for (const threadId of revalidate) {
    void getAgentState(threadId).catch(() => {});
  }
}

// ── Bulk streaming-status snapshot (no per-thread get_state fan-out) ────

/**
 * Bulk streaming-status snapshot: ONE Tauri command returns every streaming
 * thread id. The agent only scans its in-memory session map (no hydration,
 * no disk I/O). The sidebar invokes this once after installing its push
 * listener; ongoing updates arrive through `thread-streaming-updated`.
 */
export async function listStreamingThreadIds(): Promise<string[]> {
  try {
    const raw = await invokeCommand<string[]>("list_streaming_thread_ids");
    return Array.isArray(raw) ? raw : [];
  }
  catch {
    // Agent unreachable: report "nothing streaming" — the next tick retries.
    return [];
  }
}
