import type { ResolvedMarkdownReference } from "../../integrations/storage/markdownReferences";
import type { FutureReference } from "./futureMarkdownTypes";
import { useEffect, useSyncExternalStore } from "react";
import { resolveMarkdownReferences } from "../../integrations/storage/markdownReferences";

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

export function loadFutureReferences(workspaceId: string, references: ReferenceIdentity[]) {
  const workspaceLoads = pendingLoads.get(workspaceId) ?? new Map<string, ReferenceIdentity>();
  for (const reference of references) {
    workspaceLoads.set(referenceIdentityKey(reference), reference);
  }
  pendingLoads.set(workspaceId, workspaceLoads);

  if (!pendingFlush) {
    pendingFlush = setTimeout(() => {
      pendingFlush = undefined;
      void flushPendingReferenceLoads();
    }, 0);
  }
}

export function invalidateFutureReference(
  workspaceId: string | null | undefined,
  reference: ReferenceIdentity,
) {
  if (!workspaceId)
    return;

  void resolveAndStoreReferences(workspaceId, [reference]);
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

  for (const entry of entries) {
    records.set(storeKey(workspaceId, entry.targetType, entry.targetId), {
      data: entry.data,
      status: "resolved",
      targetId: entry.targetId,
      targetType: entry.targetType,
    });
  }
  pruneReferenceRecords();
  notifyFutureReferenceSubscribers();
}

export function getFutureReferenceSnapshot(
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
    const message = error instanceof Error ? error.message : String(error);
    resolved = references.map(reference => ({
      error: message,
      status: "failed",
      targetId: reference.targetId,
      targetType: reference.targetType,
    }));
  }

  for (const reference of resolved) {
    records.set(storeKey(workspaceId, reference.targetType, reference.targetId), reference);
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
