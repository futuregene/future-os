import * as pdfjs from "pdfjs-dist";
import { useEffect, useRef, useState } from "react";

// 设置 worker
pdfjs.GlobalWorkerOptions.workerSrc = new URL(
  "pdfjs-dist/build/pdf.worker.min.mjs",
  import.meta.url,
).toString();

interface PdfPreviewProps {
  path: string;
}

export function PdfPreview({ path }: PdfPreviewProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [totalPages, setTotalPages] = useState(0);
  const [currentPage, setCurrentPage] = useState(1);
  const loadingTaskRef = useRef<pdfjs.PDFDocumentLoadingTask | null>(null);

  useEffect(() => {
    let cancelled = false;

    async function loadPdf() {
      try {
        setLoading(true);
        setError(null);

        // 使用 Tauri 的 asset protocol 访问本地文件
        const url = `asset://localhost/${encodeURIComponent(path)}`;
        const loadingTask = pdfjs.getDocument({ url });
        loadingTaskRef.current = loadingTask;

        const pdf = await loadingTask.promise;

        if (cancelled) {
          await loadingTask.destroy();
          return;
        }

        setTotalPages(pdf.numPages);
        setCurrentPage(1);
        setLoading(false);
      }
      catch (err) {
        if (!cancelled) {
          setError(err instanceof Error ? err.message : "Failed to load PDF");
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
  }, [path]);

  useEffect(() => {
    const loadingTask = loadingTaskRef.current;
    if (!loadingTask || !containerRef.current || loading || error)
      return;

    let cancelled = false;

    async function renderPage() {
      try {
        if (!containerRef.current || !loadingTaskRef.current)
          return;

        // 清空容器
        containerRef.current.innerHTML = "";

        const pdf = await loadingTaskRef.current.promise;
        const page = await pdf.getPage(currentPage);
        const scale = 1.5;
        const viewport = page.getViewport({ scale });

        const canvas = document.createElement("canvas");
        canvas.height = viewport.height;
        canvas.width = viewport.width;
        canvas.style.width = "100%";
        canvas.style.height = "auto";

        containerRef.current.appendChild(canvas);

        await page.render({
          canvas,
          viewport,
        }).promise;

        if (!cancelled) {
          page.cleanup();
        }
      }
      catch (err) {
        if (!cancelled) {
          setError(err instanceof Error ? err.message : "Failed to render page");
        }
      }
    }

    void renderPage();

    return () => {
      cancelled = true;
    };
  }, [currentPage, loading, error]);

  if (loading) {
    return (
      <div className="flex items-center justify-center p-8">
        <div className="text-sm text-ink-muted">Loading PDF...</div>
      </div>
    );
  }

  if (error) {
    return (
      <div className="rounded-md border border-red-200 bg-red-50 p-3">
        <div className="text-sm font-medium text-red-900">PDF Preview Error</div>
        <div className="mt-1 text-xs text-red-700">{error}</div>
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
            ← Prev
          </button>
          <span className="text-xs text-ink-muted">
            Page
            {" "}
            {currentPage}
            {" "}
            /
            {" "}
            {totalPages}
          </span>
          <button
            onClick={() => setCurrentPage(p => Math.min(totalPages, p + 1))}
            disabled={currentPage >= totalPages}
            className="rounded-md border border-line bg-surface px-2 py-1 text-xs disabled:opacity-50"
          >
            Next →
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
