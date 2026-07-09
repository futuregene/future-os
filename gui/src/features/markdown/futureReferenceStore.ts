import type { ResolvedMarkdownReference } from "../../integrations/storage/markdownReferences";
import type { FutureReference } from "./futureMarkdownTypes";
import { useEffect, useSyncExternalStore } from "react";
import { resolveMarkdownReferences } from "../../integrations/storage/markdownReferences";
import { errorMessage } from "../../lib/errors";

interface ReferenceIdentity {
  targetId: string;
  targetType: FutureReference["targetType"];
}

type ReferenceData = ResolvedMarkdownReference["data"];

interface ReferenceStoreEntry extends ReferenceIdentity {
  data: ReferenceData;
}

const records = new Map<string, ResolvedMarkdownReference>();
const listeners = new Set<() => void>();
const pendingLoads = new Map<string, Map<string, ReferenceIdentity>>();
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
  const workspaceLoads = pendingLoads.get(workspaceId) ?? new Map<string, ReferenceIdentity>();
  for (const reference of references) {
    // The parsed `references` array gets a fresh identity on every streaming
    // delta, so this fires per keystroke. Already-resolved records stay hot via
    // ContextPanel's poll (upsertFutureReferenceEntries), so re-resolving them
    // is wasted IPC — only fetch unresolved/unknown identities.
    if (records.get(storeKey(workspaceId, reference.targetType, reference.targetId))?.status === "resolved")
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

export function upsertFutureReferenceData(
  workspaceId: string | null | undefined,
  targetType: ReferenceIdentity["targetType"],
  targetId: string,
  data: ReferenceData,
) {
  upsertFutureReferenceEntries(workspaceId, [{ data, targetId, targetType }]);
}

export function upsertFutureReferenceEntries(
  workspaceId: string | null | undefined,
  entries: ReferenceStoreEntry[],
) {
  if (!workspaceId)
    return;

  let changed = false;
  for (const entry of entries) {
    const key = storeKey(workspaceId, entry.targetType, entry.targetId);
    const existing = records.get(key);
    // Delete-then-set so `set` moves the key to the end — Map preserves
    // insertion order and does not refresh it on overwrite; keeps prune LRU.
    records.delete(key);
    if (existing?.status === "resolved" && sameReferenceData(existing.data, entry.data)) {
      // Data is byte-identical to what's already resolved (the common case on
      // ContextPanel's 1.5s poll). Re-store the SAME record object so the LRU
      // position refreshes but the snapshot reference stays stable — otherwise a
      // fresh object would make every subscribed reference chip re-render each
      // tick even though nothing changed.
      records.set(key, existing);
      continue;
    }
    records.set(key, {
      data: entry.data,
      status: "resolved",
      targetId: entry.targetId,
      targetType: entry.targetType,
    });
    changed = true;
  }
  pruneReferenceRecords();
  // Only wake subscribers when a record actually changed; a no-op poll must not
  // churn every chip.
  if (changed) {
    notifyFutureReferenceSubscribers();
  }
}

function sameReferenceData(a: ReferenceData, b: ReferenceData): boolean {
  if (a === b)
    return true;
  try {
    return JSON.stringify(a) === JSON.stringify(b);
  }
  catch {
    return false;
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

  for (const reference of resolved) {
    const key = storeKey(workspaceId, reference.targetType, reference.targetId);
    // Delete-then-set so overwrites refresh LRU order (see upsert).
    records.delete(key);
    records.set(key, reference);
  }
  pruneReferenceRecords();
  notifyFutureReferenceSubscribers();
}

function pruneReferenceRecords() {
  while (records.size > maxReferenceRecords) {
    const oldest = records.keys().next().value;
    if (!oldest)
      return;
    records.delete(oldest);
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
