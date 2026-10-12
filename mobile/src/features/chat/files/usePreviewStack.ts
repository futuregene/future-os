import { useCallback, useEffect, useRef, useState } from "react";
import { Platform } from "react-native";
import { deferPresentation } from "../utils";
import type { PreviewState } from "./useChatFiles";

export function usePreviewStack() {
  const pendingPreviewActionRef = useRef<(() => void) | null>(null);
  // A document can link another document, so previews stack. Only the top one
  // is visible; the ones below stay mounted, which is what keeps their scroll
  // position while the reader follows a link and comes back.
  const [previews, setPreviews] = useState<PreviewState[]>([]);
  const hasPreview = previews.length > 0;
  const preview = hasPreview ? previews[previews.length - 1]! : null;

  const showPreview = useCallback((state: PreviewState, nested = false) => {
    setPreviews(current => (nested ? [...current, state] : [state]));
  }, []);

  const closePreview = useCallback(() => {
    pendingPreviewActionRef.current = null;
    setPreviews([]);
  }, []);

  const popPreview = useCallback(() => {
    setPreviews(current => (current.length > 1 ? current.slice(0, -1) : current));
  }, []);

  const dismissPreviewThen = useCallback(
    (action: () => void) => {
      if (!hasPreview) {
        action();
        return;
      }
      pendingPreviewActionRef.current = action;
      setPreviews([]);
      if (Platform.OS !== "ios") {
        deferPresentation(() => {
          if (pendingPreviewActionRef.current !== action) return;
          pendingPreviewActionRef.current = null;
          action();
        });
      }
    },
    [hasPreview],
  );

  const flushPendingPreviewAction = useCallback(() => {
    if (Platform.OS !== "ios") return;
    const action = pendingPreviewActionRef.current;
    pendingPreviewActionRef.current = null;
    action?.();
  }, []);

  useEffect(() => () => { pendingPreviewActionRef.current = null; }, []);
  return { previews, preview, showPreview, closePreview, popPreview, dismissPreviewThen, flushPendingPreviewAction };
}
