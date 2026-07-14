import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { readFileBase64 } from "../../integrations/storage/files";
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
  const [src, setSrc] = useState<string | null>(null);
  // Hold onError in a ref so the load effect depends only on `path` — callers
  // often pass a fresh callback each render, which would otherwise re-fire the
  // fetch on every render and thrash the preview.
  const onErrorRef = useRef(onError);
  onErrorRef.current = onError;

  useEffect(() => {
    let cancelled = false;
    setSrc(null);
    readFileBase64({ path })
      .then((base64) => {
        if (!cancelled)
          setSrc(`data:${imageMimeForPath(path)};base64,${base64}`);
      })
      .catch(() => {
        if (!cancelled)
          onErrorRef.current();
      });
    return () => {
      cancelled = true;
    };
  }, [path]);

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
