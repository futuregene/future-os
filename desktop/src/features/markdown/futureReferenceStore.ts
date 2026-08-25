import type { ThreadRuntimeUpdateBatch } from "../../integrations/agent/runtimeEvents";
import type { ResolvedMarkdownReference } from "../../integrations/storage/markdownReferences";
import type { FutureReference } from "./futureMarkdownTypes";
import { listen } from "@tauri-apps/api/event";
import { useEffect, useSyncExternalStore } from "react";
import { resolveMarkdownReferences } from "../../integrations/storage/markdownReferences";
import { errorMessage } from "../../lib/errors";

// Lazy resolution cache for markdown reference chips (run/artifact/file
// embeds). Chips resolve on demand: `useFutureReferences` back-fills whatever
// the cache doesn't hold, batched per workspace. Resolved records stay put —
// a settled run or an artifact is an immutable reference — so no producer
// needs to keep the cache warm; the only update path is a run reaching a
// terminal status (the listener below re-resolves those records in place).
// Non-resolved records (a missing object, or a transport failure) retry on an
// escalating backoff, actively scheduled so a static page still heals — but
// only a bounded number of times: a genuinely missing/deleted/unsupported
// target parks instead of IPCing forever.

interface ReferenceIdentity {
  targetId: string;
  targetType: FutureReference["targetType"];
}

const records = new Map<string, ResolvedMarkdownReference>();
const listeners = new Set<() => void>();
const pendingLoads = new Map<string, Map<string, ReferenceIdentity>>();
// Unresolved records waiting for a retry, with the workspace they resolve
// against (the record key doesn't carry it parseably — file ids are paths).
const pendingRetry = new Map<string, { workspaceId: string; identity: ReferenceIdentity }>();
// Earliest retry time (ms) per record key; absent once a record resolves or
// its attempts are exhausted (parked).
const retryAfter = new Map<string, number>();
// Resolve attempts in the current unresolved streak (cleared on success).
const retryAttempts = new Map<string, number>();
const RETRY_BACKOFF_MS = 30_000;
// Give up after this many attempts: a genuinely missing, deleted, or
// unsupported target must not IPC forever. The races this mechanism exists
// for (a row not yet committed, a transient transport error) resolve within
// the first backoffs; later than that, the record's status is the truth.
const MAX_RETRY_ATTEMPTS = 3;
const maxReferenceRecords = 1000;
let pendingFlush: ReturnType<typeof setTimeout> | undefined;
let retryTimer: ReturnType<typeof setTimeout> | undefined;

export function useFutureReferences(workspaceId: string | null | undefined, references: FutureReference[]) {
  useEffect(() => {
    if (!workspaceId || references.length === 0)
      return;

    loadFutureReferences(workspaceId, references);
  }, [references, workspaceId]);
}

export function useFutureReference(
  workspaceId: string | null | undefined,
  reference: ReferenceIdentity,
) {
  return useSyncExternalStore(
    subscribeFutureReferences,
    () => getFutureReferenceSnapshot(workspaceId, reference),
    () => getFutureReferenceSnapshot(workspaceId, reference),
  );
}

/** Imperative counterpart of `useFutureReferences` (also used by tests). */
export function queueFutureReferenceLoad(workspaceId: string, references: ReferenceIdentity[]) {
  if (references.length === 0)
    return;
  loadFutureReferences(workspaceId, references);
}

/** Synchronous read of a cached record without subscribing. */
export function peekFutureReference(
  workspaceId: string | null | undefined,
  reference: ReferenceIdentity,
) {
  return getFutureReferenceSnapshot(workspaceId, reference);
}

function loadFutureReferences(workspaceId: string, references: ReferenceIdentity[]) {
  installTerminalRunListener();
  const now = Date.now();
  const workspaceLoads = pendingLoads.get(workspaceId) ?? new Map<string, ReferenceIdentity>();
  for (const reference of references) {
    const key = storeKey(workspaceId, reference.targetType, reference.targetId);
    const existing = records.get(key);
    // Resolved records are final (immutable objects; run records are refreshed
    // by the terminal-status listener), so they never re-resolve — the parsed
    // `references` array gets a fresh identity on every streaming delta, and
    // re-resolving hot records per keystroke would be wasted IPC.
    if (existing?.status === "resolved")
      continue;
    if (existing) {
      // Unresolved records retry on their own schedule. One that isn't due
      // yet — or is parked after exhausting its attempts (no deadline left) —
      // must not re-queue on every render.
      const due = retryAfter.get(key);
      if (due === undefined || due > now)
        continue;
    }
    workspaceLoads.set(referenceIdentityKey(reference), reference);
  }
  if (workspaceLoads.size === 0)
    return;
  pendingLoads.set(workspaceId, workspaceLoads);
  queueFlush();
}

function getFutureReferenceSnapshot(
  workspaceId: string | null | undefined,
  reference: ReferenceIdentity,
) {
  if (!workspaceId)
    return undefined;
  return records.get(storeKey(workspaceId, reference.targetType, reference.targetId));
}

function subscribeFutureReferences(listener: () => void) {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

function queueFlush() {
  if (pendingFlush)
    return;
  pendingFlush = setTimeout(() => {
    pendingFlush = undefined;
    void flushPendingReferenceLoads();
  }, 0);
}

async function flushPendingReferenceLoads() {
  const loads = [...pendingLoads.entries()];
  pendingLoads.clear();

  await Promise.all(
    loads.map(([workspaceId, references]) =>
      resolveAndStoreReferences(workspaceId, [...references.values()]),
    ),
  );
}

async function resolveAndStoreReferences(workspaceId: string, references: ReferenceIdentity[]) {
  /* v8 ignore next 2 -- invariant: flush only ever queues non-empty batches */
  if (references.length === 0)
    return;

  let resolved: ResolvedMarkdownReference[];
  try {
    resolved = await resolveMarkdownReferences(
      workspaceId,
      references.map(reference => ({
        targetId: reference.targetId,
        targetType: reference.targetType,
      })),
    );
  }
  catch (error) {
    const message = errorMessage(error);
    resolved = references.map(reference => ({
      error: message,
      status: "failed",
      targetId: reference.targetId,
      targetType: reference.targetType,
    }));
  }

  for (const reference of resolved) {
    const key = storeKey(workspaceId, reference.targetType, reference.targetId);
    // Delete-then-set so overwrites refresh LRU order (Map preserves insertion
    // order, not overwrite order).
    records.delete(key);
    records.set(key, reference);
    if (reference.status === "resolved") {
      retryAfter.delete(key);
      pendingRetry.delete(key);
      retryAttempts.delete(key);
      continue;
    }
    const attempts = (retryAttempts.get(key) ?? 0) + 1;
    retryAttempts.set(key, attempts);
    if (attempts >= MAX_RETRY_ATTEMPTS) {
      // Parked: the record keeps its missing/failed status and the chip
      // renders it as such; no further IPC.
      retryAfter.delete(key);
      pendingRetry.delete(key);
      continue;
    }
    // Escalating backoff: 30s, then 60s.
    retryAfter.set(key, Date.now() + RETRY_BACKOFF_MS * 2 ** (attempts - 1));
    pendingRetry.set(key, {
      workspaceId,
      identity: {
        targetId: reference.targetId,
        targetType: reference.targetType as ReferenceIdentity["targetType"],
      },
    });
  }
  pruneReferenceRecords();
  notifyFutureReferenceSubscribers();
  scheduleRetrySweep();
}

// ── Retry sweep ───────────────────────────────────────────────────────────
// Unresolved records re-queue themselves when their backoff elapses. The
// timer targets the earliest deadline; nothing else polls the map, so a
// static page still heals a record that first resolved missing (row not yet
// committed) or failed (transient IPC error).

function scheduleRetrySweep() {
  let earliest = Number.POSITIVE_INFINITY;
  for (const key of pendingRetry.keys()) {
    const due = retryAfter.get(key);
    if (due !== undefined && due < earliest)
      earliest = due;
  }
  if (!Number.isFinite(earliest)) {
    if (retryTimer !== undefined) {
      clearTimeout(retryTimer);
      retryTimer = undefined;
    }
    return;
  }
  // A new earlier deadline replaces a later scheduled sweep.
  if (retryTimer !== undefined)
    clearTimeout(retryTimer);
  retryTimer = setTimeout(runRetrySweep, Math.max(0, earliest - Date.now()));
}

function runRetrySweep() {
  retryTimer = undefined;
  const now = Date.now();
  let queued = false;
  for (const [key, entry] of [...pendingRetry]) {
    if ((retryAfter.get(key) ?? 0) > now)
      continue;
    pendingRetry.delete(key);
    const workspaceLoads = pendingLoads.get(entry.workspaceId) ?? new Map<string, ReferenceIdentity>();
    workspaceLoads.set(referenceIdentityKey(entry.identity), entry.identity);
    pendingLoads.set(entry.workspaceId, workspaceLoads);
    queued = true;
  }
  if (queued)
    queueFlush();
  // Re-arm for deadlines that aren't due yet.
  if (pendingRetry.size > 0)
    scheduleRetrySweep();
}

// ── Terminal-status refresh ───────────────────────────────────────────────
// A run's record (status, tokens, error) changes one last time when it
// settles. The push names the run, so re-resolve exactly those records in
// place — deleting them instead would flash the chip to its pending state.

let terminalListenerInstalled = false;

function installTerminalRunListener() {
  if (terminalListenerInstalled)
    return;
  terminalListenerInstalled = true;
  void listen<ThreadRuntimeUpdateBatch>("thread-runtime-updated", (event) => {
    for (const { runId, status } of event.payload.updates) {
      if (!["completed", "failed", "cancelled"].includes(status))
        continue;
      void reresolveRunRecords(runId);
    }
  });
}

async function reresolveRunRecords(runId: string) {
  const suffix = `:run:${runId}`;
  // Group the affected records by workspace (the key prefix) so each
  // workspace gets one batched resolve call.
  const byWorkspace = new Map<string, ReferenceIdentity>();
  for (const key of records.keys()) {
    if (!key.endsWith(suffix))
      continue;
    const workspaceId = key.slice(0, key.length - suffix.length);
    byWorkspace.set(workspaceId, { targetId: runId, targetType: "run" });
  }
  await Promise.all(
    [...byWorkspace.entries()].map(([workspaceId, identity]) =>
      resolveAndStoreReferences(workspaceId, [identity]),
    ),
  );
}

function pruneReferenceRecords() {
  while (records.size > maxReferenceRecords) {
    // size > maxReferenceRecords (>= 1) guarantees a first key exists.
    const oldest = records.keys().next().value!;
    records.delete(oldest);
    retryAfter.delete(oldest);
    pendingRetry.delete(oldest);
    retryAttempts.delete(oldest);
  }
}

function notifyFutureReferenceSubscribers() {
  for (const listener of listeners) {
    listener();
  }
}

function referenceIdentityKey(reference: ReferenceIdentity) {
  return `${reference.targetType}:${reference.targetId}`;
}

function storeKey(workspaceId: string, targetType: string, targetId: string) {
  return `${workspaceId}:${targetType}:${targetId}`;
}
