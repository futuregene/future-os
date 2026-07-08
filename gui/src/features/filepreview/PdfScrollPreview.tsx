import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { readFileBase64 } from "../../integrations/storage/files";
import { base64ToBytes, PDF_WASM_URL, pdfjs } from "./pdfjsSetup";
import { PreviewNotice } from "./PreviewNotice";

// A PDF point is 1/72in; screens are ~96dpi, so scale=96/72 is "100%" (natural
// size). We never render wider than that, so previews shrink to fit but never
// upscale — matching the image behavior.
const NATURAL_SCALE = 96 / 72;

/**
 * Renders every page of a local PDF stacked vertically for scroll-through
 * reading (no pagination). Bytes come through the backend (`read_file_base64`,
 * 25MB cap) so out-of-workspace paths preview without widening the asset-protocol
 * scope. Canvases are appended to a ref'd div that React never owns, so a state
 * update can't wipe painted pages; a load failure routes to `onError`.
 */
export function PdfScrollPreview({
  path,
  onError,
}: {
  path: string;
  onError: () => void;
}) {
  const { t } = useTranslation("markdown");
  const containerRef = useRef<HTMLDivElement>(null);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    let cancelled = false;
    let loadingTask: pdfjs.PDFDocumentLoadingTask | null = null;

    async function render() {
      try {
        setLoading(true);
        const base64 = await readFileBase64({ path });
        if (cancelled)
          return;

        loadingTask = pdfjs.getDocument({ data: base64ToBytes(base64), wasmUrl: PDF_WASM_URL });
        const pdf = await loadingTask.promise;
        if (cancelled) {
          await loadingTask.destroy();
          return;
        }

        const container = containerRef.current;
        if (!container)
          return;
        container.replaceChildren();

        const dpr = window.devicePixelRatio || 1;
        const available = container.clientWidth || 0;

        for (let pageNumber = 1; pageNumber <= pdf.numPages; pageNumber += 1) {
          if (cancelled)
            return;
          const page = await pdf.getPage(pageNumber);
          const unscaled = page.getViewport({ scale: 1 });
          // Fit width to the container but never past natural (96dpi) size.
          const cssWidth = available > 0
            ? Math.min(unscaled.width * NATURAL_SCALE, available)
            : unscaled.width * NATURAL_SCALE;
          const scale = cssWidth / unscaled.width;
          const viewport = page.getViewport({ scale: scale * dpr });

          const canvas = document.createElement("canvas");
          canvas.width = viewport.width;
          canvas.height = viewport.height;
          canvas.style.width = `${cssWidth}px`;
          canvas.style.height = "auto";
          canvas.className = "mx-auto block rounded-md bg-surface shadow-panel";
          if (cancelled)
            return;
          container.appendChild(canvas);

          await page.render({ canvas, viewport }).promise;
          page.cleanup();
        }

        if (!cancelled)
          setLoading(false);
      }
      catch {
        if (!cancelled)
          onError();
      }
    }

    void render();

    return () => {
      cancelled = true;
      if (loadingTask)
        void loadingTask.destroy();
    };
  }, [path, onError]);

  return (
    <div className="relative w-full">
      {loading ? <PreviewNotice message={t("filePreview.loading")} /> : null}
      <div className="flex flex-col items-center gap-4" ref={containerRef} />
    </div>
  );
}
