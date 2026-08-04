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
// Non-resolved records (a missing object, or a transport failure) retry after
// a backoff so a race (e.g. a run row not committed yet) heals without
// re-firing IPC on every streaming delta.

interface ReferenceIdentity {
  targetId: string;
  targetType: FutureReference["targetType"];
}

const records = new Map<string, ResolvedMarkdownReference>();
const listeners = new Set<() => void>();
const pendingLoads = new Map<string, Map<string, ReferenceIdentity>>();
// Earliest retry time (ms) for non-resolved records.
const retryAfter = new Map<string, number>();
const RETRY_BACKOFF_MS = 30_000;
const maxReferenceRecords = 1000;
let pendingFlush: ReturnType<typeof setTimeout> | undefined;

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
    // A missing object or a failed transport retries after the backoff, not on
    // every render — but it DOES retry, so a record that missed a row not yet
    // committed (or a transient IPC error) heals on its own.
    if (existing && (retryAfter.get(key) ?? 0) > now)
      continue;
    workspaceLoads.set(referenceIdentityKey(reference), reference);
  }
  if (workspaceLoads.size === 0)
    return;
  pendingLoads.set(workspaceId, workspaceLoads);

  if (!pendingFlush) {
    pendingFlush = setTimeout(() => {
      pendingFlush = undefined;
      void flushPendingReferenceLoads();
    }, 0);
  }
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

  const retryAt = Date.now() + RETRY_BACKOFF_MS;
  for (const reference of resolved) {
    const key = storeKey(workspaceId, reference.targetType, reference.targetId);
    // Delete-then-set so overwrites refresh LRU order (Map preserves insertion
    // order, not overwrite order).
    records.delete(key);
    records.set(key, reference);
    if (reference.status === "resolved")
      retryAfter.delete(key);
    else
      retryAfter.set(key, retryAt);
  }
  pruneReferenceRecords();
  notifyFutureReferenceSubscribers();
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
  void listen<{ runId?: string; status?: string }>("thread-runtime-updated", (event) => {
    const { runId, status } = event.payload ?? {};
    if (!runId || !status || !["completed", "failed", "cancelled"].includes(status))
      return;
    void reresolveRunRecords(runId);
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
    const oldest = records.keys().next().value;
    if (!oldest)
      return;
    records.delete(oldest);
    retryAfter.delete(oldest);
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
