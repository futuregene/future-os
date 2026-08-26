import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { prepareImagePreviewUrl } from "../../integrations/storage/files";
import { useAsyncResource } from "../../lib/useAsyncResource";
import { PreviewNotice } from "./PreviewNotice";
import { usePreviewLoadingGate } from "./usePreviewLoadingGate";

/**
 * Renders a local image at its natural size, shrinking to fit the overlay when
 * larger (`max-h/w-full`) and never upscaling. Bytes come through the backend
 * through a path-validated Tauri asset URL (25MB cap), so paths outside the
 * workspace still preview without copying or Base64-encoding their bytes.
 */
export function ImagePreview({
  path,
  name,
  onError,
}: {
  path: string;
  name: string;
  onError: () => void;
}) {
  const { t } = useTranslation("markdown");
  const { data: src, error, loading } = useAsyncResource<string | null>(
    () => prepareImagePreviewUrl(path),
    [path],
    null,
  );
  // Hold onError in a ref so the failure effect doesn't re-fire when callers
  // pass a fresh callback each render.
  const onErrorRef = useRef(onError);
  onErrorRef.current = onError;

  const readySrc = loading || error ? null : src;
  const [loadedSrc, setLoadedSrc] = useState<string | null>(null);
  const [failedSrc, setFailedSrc] = useState<string | null>(null);
  const imageLoaded = readySrc !== null && loadedSrc === readySrc;
  const imageFailed = readySrc !== null && failedSrc === readySrc;
  const gate = usePreviewLoadingGate(loading || (readySrc !== null && !imageLoaded && !imageFailed));
  const failed = Boolean(error || imageFailed);

  // A read failure (missing/too large) routes back to `onError` so the overlay
  // falls back to the OS default handler. If the loading notice became visible,
  // wait for its minimum display window before closing the overlay.
  useEffect(() => {
    if (failed && gate.showContent)
      onErrorRef.current();
  }, [failed, gate.showContent]);

  if (!readySrc && !gate.showLoading)
    return null;

  return (
    <>
      {gate.showLoading ? <PreviewNotice message={t("filePreview.loading")} /> : null}
      {readySrc
        ? (
            // Keep the image mounted while hidden so browser decoding is part
            // of the loading interval, not a blank frame after it.
            <img
              alt={name}
              aria-hidden={!gate.showContent || failed}
              className={gate.showContent && !failed
                ? "visible max-h-[calc(100vh-4rem)] max-w-[calc(100vw-4rem)] rounded-md object-contain shadow-panel"
                : "fixed left-0 top-0 size-px invisible"}
              onError={() => setFailedSrc(readySrc)}
              onLoad={() => setLoadedSrc(readySrc)}
              src={readySrc}
            />
          )
        : null}
    </>
  );
}
