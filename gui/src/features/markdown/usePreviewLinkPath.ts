import { useEffect, useState } from "react";
import { resolvePreviewLinkPath } from "../../integrations/storage/files";

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
  const [resolved, setResolved] = useState<PreviewLinkPath | null>(null);

  useEffect(() => {
    let cancelled = false;
    setResolved(null);
    resolvePreviewLinkPath(baseFile, target)
      .then((result) => {
        if (!cancelled)
          setResolved(result);
      })
      .catch(() => {
        if (!cancelled)
          setResolved(null);
      });
    return () => {
      cancelled = true;
    };
  }, [baseFile, target]);

  return resolved;
}
