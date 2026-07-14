import { useEffect, useRef } from "react";
import { useTranslation } from "react-i18next";
import { readFileBase64 } from "../../integrations/storage/files";
import { useAsyncResource } from "../../lib/useAsyncResource";
import { imageMimeForPath } from "./previewKind";
import { PreviewNotice } from "./PreviewNotice";

/**
 * Renders a local image at its natural size, shrinking to fit the overlay when
 * larger (`max-h/w-full`) and never upscaling. Bytes come through the backend
 * (`read_file_base64`, 25MB cap) rather than the asset protocol, so paths
 * outside the workspace still preview; a read failure (missing/too large) routes
 * back to `onError`.
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
  const { data: base64, error, loading } = useAsyncResource<string | null>(
    () => readFileBase64({ path }),
    [path],
    null,
  );
  // Hold onError in a ref so the failure effect doesn't re-fire when callers
  // pass a fresh callback each render.
  const onErrorRef = useRef(onError);
  onErrorRef.current = onError;

  // A read failure (missing/too large) routes back to `onError` so the overlay
  // falls back to the OS default handler.
  useEffect(() => {
    if (error)
      onErrorRef.current();
  }, [error]);

  const src = loading || error || base64 == null
    ? null
    : `data:${imageMimeForPath(path)};base64,${base64}`;

  if (!src)
    return <PreviewNotice message={t("filePreview.loading")} />;

  return (
    // Cap against the viewport (a definite length) rather than `max-h/w-full`:
    // the image's parent is content-sized, so a percentage max would resolve to
    // "no limit" and the image would overflow. The inset leaves breathing room.
    <img
      alt={name}
      className="max-h-[calc(100vh-4rem)] max-w-[calc(100vw-4rem)] rounded-md object-contain shadow-panel"
      onError={onError}
      src={src}
    />
  );
}
