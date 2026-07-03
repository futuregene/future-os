import { convertFileSrc } from "@tauri-apps/api/core";
import * as pdfjs from "pdfjs-dist";
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";

// 设置 worker
pdfjs.GlobalWorkerOptions.workerSrc = new URL(
  "pdfjs-dist/build/pdf.worker.min.mjs",
  import.meta.url,
).toString();

// pdf.js v6 decodes JBIG2/JPEG2000/CCITT scanned images via WebAssembly loaded from
// this base dir (served by the `pdfjsWasm` Vite plugin). Without it, image-only PDFs
// render as a blank page. Absolute href so pdf.js's `new URL(name, wasmUrl)` resolves.
const PDF_WASM_URL = new URL("pdfjs-wasm/", document.baseURI).href;

interface PdfPreviewProps {
  path: string;
}

export function PdfPreview({ path }: PdfPreviewProps) {
  const { t } = useTranslation("artifacts");
  const containerRef = useRef<HTMLDivElement>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [totalPages, setTotalPages] = useState(0);
  const [currentPage, setCurrentPage] = useState(1);
  const [pdfDoc, setPdfDoc] = useState<pdfjs.PDFDocumentProxy | null>(null);
  const loadingTaskRef = useRef<pdfjs.PDFDocumentLoadingTask | null>(null);

  useEffect(() => {
    let cancelled = false;

    async function loadPdf() {
      try {
        setLoading(true);
        setError(null);

        // Tauri asset protocol (convertFileSrc handles per-OS scheme + Windows).
        const url = convertFileSrc(path);
        const loadingTask = pdfjs.getDocument({ url, wasmUrl: PDF_WASM_URL });
        loadingTaskRef.current = loadingTask;

        const pdf = await loadingTask.promise;

        if (cancelled) {
          await loadingTask.destroy();
          return;
        }

        setPdfDoc(pdf);
        setTotalPages(pdf.numPages);
        setCurrentPage(1);
        setLoading(false);
      }
      catch (err) {
        if (!cancelled) {
          setError(err instanceof Error ? err.message : t("pdfPreview.failedToLoad"));
          setLoading(false);
        }
      }
    }

    void loadPdf();

    return () => {
      cancelled = true;
      if (loadingTaskRef.current) {
        loadingTaskRef.current.destroy();
        loadingTaskRef.current = null;
      }
    };
  }, [path, t]);

  useEffect(() => {
    const container = containerRef.current;
    if (!pdfDoc || !container)
      return;

    let cancelled = false;

    async function renderPage() {
      try {
        const page = await pdfDoc!.getPage(currentPage);
        if (cancelled)
          return;

        const scale = 1.5;
        const viewport = page.getViewport({ scale });

        const canvas = document.createElement("canvas");
        canvas.height = viewport.height;
        canvas.width = viewport.width;
        canvas.style.width = "100%";
        canvas.style.height = "auto";

        // Clear + attach only once we're about to paint, and bail if a newer
        // render (page/document change) superseded this one — otherwise a late
        // resolve could blank the container or paint a stale page.
        if (cancelled || !container)
          return;
        container.innerHTML = "";
        container.appendChild(canvas);

        await page.render({ canvas, viewport }).promise;

        if (!cancelled) {
          page.cleanup();
        }
      }
      catch (err) {
        if (!cancelled) {
          setError(err instanceof Error ? err.message : t("pdfPreview.failedToRender"));
        }
      }
    }

    void renderPage();

    return () => {
      cancelled = true;
    };
  }, [currentPage, pdfDoc, t]);

  if (loading) {
    return (
      <div className="flex items-center justify-center p-8">
        <div className="text-sm text-ink-muted">{t("pdfPreview.loading")}</div>
      </div>
    );
  }

  if (error) {
    return (
      <div className="rounded-md border border-danger-line bg-danger-soft p-3">
        <div className="text-sm font-medium text-danger">{t("pdfPreview.errorTitle")}</div>
        <div className="mt-1 text-xs text-danger">{error}</div>
      </div>
    );
  }

  return (
    <div className="space-y-3">
      <div className="flex items-center justify-between gap-2">
        <div className="flex items-center gap-2">
          <button
            onClick={() => setCurrentPage(p => Math.max(1, p - 1))}
            disabled={currentPage <= 1}
            className="rounded-md border border-line bg-surface px-2 py-1 text-xs disabled:opacity-50"
          >
            {t("pdfPreview.prev")}
          </button>
          <span className="text-xs text-ink-muted">
            {t("pdfPreview.pageIndicator", { current: currentPage, total: totalPages })}
          </span>
          <button
            onClick={() => setCurrentPage(p => Math.min(totalPages, p + 1))}
            disabled={currentPage >= totalPages}
            className="rounded-md border border-line bg-surface px-2 py-1 text-xs disabled:opacity-50"
          >
            {t("pdfPreview.next")}
          </button>
        </div>
      </div>
      <div
        ref={containerRef}
        className="overflow-auto rounded-md border border-line bg-surface"
        style={{ maxHeight: "600px" }}
      />
    </div>
  );
}
