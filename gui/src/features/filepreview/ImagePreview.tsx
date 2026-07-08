import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { readFileBase64 } from "../../integrations/storage/files";
import { imageMimeForPath } from "./previewKind";
import { PreviewNotice } from "./PreviewNotice";

/**
 * Renders a local image at its natural size, shrinking to fit the overlay when
 * larger (`max-h/w-full`) and never upscaling. Bytes come through the backend
 * (`read_file_base64`, 25MB cap) rather than the asset protocol, so paths
 * outside the workspace still preview; a read failure (missing/too large) routes
 * back to `onError` for the open-externally fallback.
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
          onError();
      });
    return () => {
      cancelled = true;
    };
  }, [path, onError]);

  if (!src)
    return <PreviewNotice message={t("filePreview.loading")} />;

  return (
    <img
      alt={name}
      className="max-h-full max-w-full rounded-md object-contain shadow-panel"
      onError={onError}
      src={src}
    />
  );
}
