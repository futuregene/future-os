import type { Size, ZoomState } from "./imageZoom";
import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { prepareImagePreviewUrl } from "../../integrations/storage/files";
import { cn } from "../../lib/cn";
import { useAsyncResource } from "../../lib/useAsyncResource";
import {
  clampScale,
  clampTranslation,
  fitScale,
  MIN_SCALE,
  resetZoom,
  zoomAtAnchor,
} from "./imageZoom";
import { PreviewNotice } from "./PreviewNotice";
import { usePreviewLoadingGate } from "./usePreviewLoadingGate";

/**
 * Wheel delta that feels comfortable across mice (line) and trackpads (pixel).
 * A single wheel "click" zooms by ~10%; fast trackpad swipes scale more.
 */
const WHEEL_SENSITIVITY = 0.002;

/** Distance a drag must travel before it is considered a pan rather than a click. */
const DRAG_THRESHOLD_PX = 4;

/**
 * Renders a local image at its natural size, shrinking to fit the overlay when
 * larger (`max-h/w-full`) and never upscaling. Bytes come through the backend
 * through a path-validated Tauri asset URL (25MB cap), so paths outside the
 * workspace still preview without copying or Base64-encoding their bytes.
 *
 * Desktop interaction added on top of the fit-to-window baseline:
 * - Wheel/trackpad scroll zooms in and out, anchored at the cursor.
 * - Double-click toggles between "fit to window" and "actual size" (100%).
 * - On-screen controls zoom in/out/reset and are keyboard-focusable.
 * - When zoomed in, drag to pan; panning is clamped to the image bounds so the
 *   picture cannot be lost.
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

  // Zoom / pan state.
  const containerRef = useRef<HTMLDivElement>(null);
  const [containerSize, setContainerSize] = useState<Size>({ width: 0, height: 0 });
  const [naturalSize, setNaturalSize] = useState<Size>({ width: 0, height: 0 });
  const [zoom, setZoom] = useState<ZoomState>(resetZoom);
  const [isDragging, setIsDragging] = useState(false);
  const dragStartRef = useRef<{ x: number; y: number; originX: number; originY: number } | null>(null);
  const hasPannedRef = useRef(false);

  // Reset zoom whenever the image (path/src) changes or the overlay closes.
  useEffect(() => {
    setZoom(resetZoom());
  }, [path, src]);

  // Observe container size so the zoom math stays correct across resizes.
  //
  // This effect must re-run once the container exists: on the first commit the
  // overlay renders *nothing* (the bytes are still being read, and the loading
  // notice has not appeared yet), so `containerRef.current` is null and the
  // measurement — the observer and the resize listener with it — was skipped for
  // the rest of the mount. With no box measured, `fit` is 0, every zoom anchor
  // is computed against a 0x0 container and the transform becomes
  // `scale(NaN)` (dropped by the browser), so double-click/zoom/pan silently did
  // nothing until something else happened to resize the window.
  useEffect(() => {
    const el = containerRef.current;
    if (!el)
      return;

    const update = () => {
      const rect = el.getBoundingClientRect();
      setContainerSize({ width: rect.width, height: rect.height });
    };
    update();

    // `ResizeObserver` is not present in every embedder (jsdom, for one): the
    // initial `update()` above is what the zoom math needs at all, and the
    // window `resize` listener below still follows the overlay. Same guard as
    // `useComposerInset` / `useStickyAutoScroll`.
    const observer = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(update);
    observer?.observe(el);
    window.addEventListener("resize", update);
    return () => {
      observer?.disconnect();
      window.removeEventListener("resize", update);
    };
  }, [readySrc]);

  const fit = fitScale(containerSize, naturalSize);
  const actualScale = fit * zoom.scale;

  // Re-clamp translation when the container or image size changes.
  useEffect(() => {
    setZoom(prev => ({
      ...prev,
      x: clampTranslation(prev.x, prev.scale, containerSize.width, naturalSize.width * fit),
      y: clampTranslation(prev.y, prev.scale, containerSize.height, naturalSize.height * fit),
    }));
  }, [containerSize, naturalSize, fit]);

  // A read failure (missing/too large) routes back to `onError` so the overlay
  // falls back to the OS default handler. If the loading notice became visible,
  // wait for its minimum display window before closing the overlay.
  useEffect(() => {
    if (failed && gate.showContent)
      onErrorRef.current();
  }, [failed, gate.showContent]);

  const handleWheel = useCallback((event: React.WheelEvent<HTMLDivElement>) => {
    event.preventDefault();
    const rect = containerRef.current?.getBoundingClientRect();
    if (!rect)
      return;
    const anchor = { x: event.clientX - rect.left, y: event.clientY - rect.top };
    const delta = -event.deltaY * WHEEL_SENSITIVITY;
    setZoom((prev) => {
      const nextScale = clampScale(prev.scale * (1 + delta));
      return zoomAtAnchor(containerSize, naturalSize, prev, anchor, nextScale);
    });
  }, [containerSize, naturalSize]);

  const handleDoubleClick = useCallback((event: React.MouseEvent<HTMLDivElement>) => {
    const rect = containerRef.current?.getBoundingClientRect();
    if (!rect)
      return;
    const anchor = { x: event.clientX - rect.left, y: event.clientY - rect.top };
    setZoom((prev) => {
      // Toggle between "fit" and "actual size" (100% of the natural pixels).
      const target = prev.scale > MIN_SCALE ? MIN_SCALE : 1 / fit;
      return zoomAtAnchor(containerSize, naturalSize, prev, anchor, target);
    });
  }, [containerSize, naturalSize, fit]);

  const handleMouseDown = useCallback((event: React.MouseEvent<HTMLDivElement>) => {
    // Only left clicks initiate a pan; ignore if the user clicked a control.
    if (event.button !== 0 || event.defaultPrevented)
      return;
    event.preventDefault();
    setIsDragging(true);
    hasPannedRef.current = false;
    dragStartRef.current = {
      x: event.clientX,
      y: event.clientY,
      originX: zoom.x,
      originY: zoom.y,
    };
  }, [zoom.x, zoom.y]);

  const handleMouseMove = useCallback((event: React.MouseEvent<HTMLDivElement>) => {
    if (!isDragging || !dragStartRef.current)
      return;
    const dx = event.clientX - dragStartRef.current.x;
    const dy = event.clientY - dragStartRef.current.y;
    if (Math.abs(dx) > DRAG_THRESHOLD_PX || Math.abs(dy) > DRAG_THRESHOLD_PX) {
      hasPannedRef.current = true;
    }
    setZoom(prev => ({
      ...prev,
      x: clampTranslation(dragStartRef.current!.originX + dx, prev.scale, containerSize.width, naturalSize.width * fit),
      y: clampTranslation(dragStartRef.current!.originY + dy, prev.scale, containerSize.height, naturalSize.height * fit),
    }));
  }, [isDragging, containerSize, naturalSize, fit]);

  const handleMouseUp = useCallback(() => {
    setIsDragging(false);
    dragStartRef.current = null;
  }, []);

  // Attach / detach global mouseup so a drag that leaves the window still ends.
  useEffect(() => {
    if (!isDragging)
      return;
    window.addEventListener("mouseup", handleMouseUp);
    return () => window.removeEventListener("mouseup", handleMouseUp);
  }, [isDragging, handleMouseUp]);

  const handleZoomIn = useCallback(() => {
    const rect = containerRef.current?.getBoundingClientRect();
    const anchor = rect ? { x: rect.width / 2, y: rect.height / 2 } : { x: 0, y: 0 };
    setZoom(prev => zoomAtAnchor(containerSize, naturalSize, prev, anchor, prev.scale * 1.25));
  }, [containerSize, naturalSize]);

  const handleZoomOut = useCallback(() => {
    const rect = containerRef.current?.getBoundingClientRect();
    const anchor = rect ? { x: rect.width / 2, y: rect.height / 2 } : { x: 0, y: 0 };
    setZoom(prev => zoomAtAnchor(containerSize, naturalSize, prev, anchor, prev.scale / 1.25));
  }, [containerSize, naturalSize]);

  const handleReset = useCallback(() => {
    setZoom(resetZoom());
  }, []);

  if (!readySrc && !gate.showLoading)
    return null;

  const imageLoadedClass = gate.showContent && !failed ? "visible" : "invisible";

  return (
    <div
      ref={containerRef}
      className="relative z-10 flex max-h-full max-w-full flex-col items-center justify-center outline-none"
      onDoubleClick={handleDoubleClick}
      onMouseDown={handleMouseDown}
      onMouseMove={handleMouseMove}
      onMouseUp={handleMouseUp}
      onWheel={handleWheel}
      role="presentation"
    >
      {gate.showLoading ? <PreviewNotice message={t("filePreview.loading")} /> : null}
      {readySrc
        ? (
            // Keep the image mounted while hidden so browser decoding is part
            // of the loading interval, not a blank frame after it.
            <img
              alt={name}
              aria-hidden={!gate.showContent || failed}
              className={cn(
                "max-h-[calc(100vh-4rem)] max-w-[calc(100vw-4rem)] select-none rounded-md object-contain shadow-panel",
                imageLoadedClass,
                isDragging && "cursor-grabbing",
                !isDragging && zoom.scale > MIN_SCALE && "cursor-grab",
              )}
              onError={() => setFailedSrc(readySrc)}
              onLoad={(event) => {
                setLoadedSrc(readySrc);
                setNaturalSize({
                  width: event.currentTarget.naturalWidth,
                  height: event.currentTarget.naturalHeight,
                });
              }}
              src={readySrc}
              style={{
                transform: `translate(${zoom.x}px, ${zoom.y}px) scale(${actualScale / fit})`,
                transformOrigin: "center center",
              }}
            />
          )
        : null}
      {gate.showContent && !failed && readySrc
        ? (
            <ZoomControls
              onReset={handleReset}
              onZoomIn={handleZoomIn}
              onZoomOut={handleZoomOut}
              scale={zoom.scale}
              t={t}
            />
          )
        : null}
    </div>
  );
}

function ZoomControls({
  onZoomIn,
  onZoomOut,
  onReset,
  scale,
  t,
}: {
  onZoomIn: () => void;
  onZoomOut: () => void;
  onReset: () => void;
  scale: number;
  t: (key: string) => string;
}) {
  return (
    <div className="fixed bottom-6 left-1/2 z-20 flex -translate-x-1/2 items-center gap-2 rounded-full border border-line-soft bg-surface/90 p-1.5 shadow-panel">
      <IconButton label={t("filePreview.zoomIn")} onClick={onZoomIn}>
        <ZoomIcon type="in" />
      </IconButton>
      <IconButton label={t("filePreview.zoomOut")} onClick={onZoomOut}>
        <ZoomIcon type="out" />
      </IconButton>
      <IconButton label={t("filePreview.resetZoom")} onClick={onReset}>
        <ZoomIcon type="reset" />
      </IconButton>
      <span className="px-2 text-xs tabular-nums text-ink-muted">
        {Math.round(scale * 100)}
        %
      </span>
    </div>
  );
}

function IconButton({
  label,
  onClick,
  children,
}: {
  label: string;
  onClick: () => void;
  children: React.ReactNode;
}) {
  return (
    <button
      aria-label={label}
      className="inline-flex size-8 items-center justify-center rounded-full text-ink-soft transition-colors hover:bg-surface-subtle hover:text-ink focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-focus"
      onClick={onClick}
      title={label}
      type="button"
    >
      {children}
    </button>
  );
}

function ZoomIcon({ type }: { type: "in" | "out" | "reset" }) {
  if (type === "reset") {
    return (
      <svg className="size-4" fill="none" stroke="currentColor" strokeWidth="2" viewBox="0 0 24 24">
        <path d="M3 12a9 9 0 1 1 3 7.5" />
        <path d="M3 12h6" />
      </svg>
    );
  }
  return (
    <svg className="size-4" fill="none" stroke="currentColor" strokeWidth="2" viewBox="0 0 24 24">
      {type === "in"
        ? (
            <>
              <path d="M12 5v14M5 12h14" />
            </>
          )
        : (
            <>
              <path d="M5 12h14" />
            </>
          )}
    </svg>
  );
}
