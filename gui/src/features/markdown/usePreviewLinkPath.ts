import { resolvePreviewLinkPath } from "../../integrations/storage/files";
import { useAsyncResource } from "../../lib/useAsyncResource";

export interface PreviewLinkPath {
  path: string;
  name: string;
}

/**
 * Resolve a preview-mode local link `target` against `baseFile`'s directory via
 * the backend (cross-platform path arithmetic). Returns `null` while the resolve
 * is in flight or if it fails — the caller shows a neutral placeholder until the
 * absolute path is known.
 */
export function usePreviewLinkPath(baseFile: string, target: string): PreviewLinkPath | null {
  const { data, error, loading } = useAsyncResource<PreviewLinkPath | null>(
    () => resolvePreviewLinkPath(baseFile, target),
    [baseFile, target],
    null,
  );
  // Neutral placeholder while the resolve is in flight or if it failed.
  return loading || error ? null : data;
}
