import { useCallback, useState } from "react";
import { useCommittedRef } from "../../lib/useCommittedRef";

/** Progress belongs to one visit; late updates cannot hide a newer transfer. */
export function useTransferProgress(scope: object) {
  const [progress, setProgress] = useState<{ scope: object; value: number | null; } | null>(null);
  const currentScope = useCommittedRef(scope);
  const setTransferProgress = useCallback((value: number | null) => {
    if (currentScope.current !== scope) return;
    setProgress({ scope, value });
  }, [currentScope, scope]);

  return {
    transferProgress: progress?.scope === scope ? progress.value : null,
    setTransferProgress,
  };
}
