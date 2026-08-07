import type { ResolvedMarkdownReference } from "../../../integrations/storage/markdownReferences";
import type { FutureReference } from "../futureMarkdownTypes";
import { isStoredFile } from "../../../integrations/storage/typeGuards";
import { FileLink } from "./FileLink";
import { PendingReference } from "./PendingReference";

/**
 * File references render as a link for both inline and block forms, and never as
 * a red "missing" badge: resolution is pure path arithmetic so it always
 * succeeds. Returns null for non-file references (let the caller handle those);
 * while a file reference is still resolving, shows a neutral text placeholder.
 */
export function renderFileReference(reference: FutureReference, resolved?: ResolvedMarkdownReference) {
  if (reference.targetType !== "file")
    return null;
  if (resolved?.status === "resolved" && resolved.targetType === "file" && isStoredFile(resolved.data))
    return <FileLink file={resolved.data} />;
  return <PendingReference reference={reference} />;
}
