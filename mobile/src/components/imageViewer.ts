import { createContext, useContext } from "react";

/**
 * One picture, as the viewer needs it: bytes it can draw and a name to show in
 * its title. `name` is a hint only — an inline reply image may have nothing but
 * its alt text.
 */
export interface ImageViewerTarget {
  uri: string;
  name?: string;
}

/**
 * Opens the full-screen image viewer (pinch to zoom, drag to pan).
 *
 * Context rather than a prop: pictures reach the transcript through the Markdown
 * pipeline (`MarkdownText` → `MarkdownBlock` → `MarkdownImage`) and through the
 * document preview, and none of those layers has anything else to say about a
 * viewer. A missing provider means "this surface has no viewer" — the image then
 * simply is not tappable, which is what every non-conversation surface wants.
 */
export const ImageViewerContext = createContext<((target: ImageViewerTarget) => void) | null>(null);

export function useImageViewer(): ((target: ImageViewerTarget) => void) | null {
  return useContext(ImageViewerContext);
}
