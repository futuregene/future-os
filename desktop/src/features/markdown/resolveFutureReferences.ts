import type { FutureReference, ResolvedReferenceMap } from "./futureMarkdownTypes";
import { resolveMarkdownReferences } from "../../integrations/storage/markdownReferences";
import { referenceKey } from "./futureMarkdownTypes";

export async function resolveFutureReferences(workspaceId: string, references: FutureReference[]) {
  const uniqueReferences = [...new Map(references.map(reference => [referenceKey(reference), reference])).values()];
  const resolved = await resolveMarkdownReferences(
    workspaceId,
    uniqueReferences.map(reference => ({
      targetId: reference.targetId,
      targetType: reference.targetType,
    })),
  );
  return Object.fromEntries(resolved.map(reference => [`${reference.targetType}:${reference.targetId}`, reference])) as ResolvedReferenceMap;
}
