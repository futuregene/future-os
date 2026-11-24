import type { GalleryImage, ImageGallery } from "./imageGallery";
import { createContext, useContext } from "react";

/**
 * The drawing a picture belongs to, and where in it the reader is: the entries
 * are one reply's pictures in document order, so stepping can move to either
 * neighbour and wrap at both ends.
 */
export interface ImageViewerGallery {
  entries: GalleryImage[];
  index: number;
}

/**
 * One picture, as the viewer needs it: bytes it can draw and a name to show in
 * its title. `name` is a hint only — an inline reply image may have nothing but
 * its alt text. `gallery` is present when the picture was reached from a reply
 * that drew several.
 */
export interface ImageViewerTarget {
  uri: string;
  name?: string;
  gallery?: ImageViewerGallery;
}

/**
 * Opens the full-screen image viewer (pinch to zoom, drag to pan, swipe between
 * a drawing's pictures).
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

/**
 * The target for a tap on one picture: the drawing's other pictures ride along
 * so the viewer can step through them. A surface with no gallery opens exactly
 * the picture it holds, as before.
 */
export function galleryTarget(
  gallery: ImageGallery | null,
  id: string,
  fallback: { name?: string; uri: string },
): ImageViewerTarget {
  const entries = gallery?.snapshot() ?? [];
  const index = entries.findIndex(image => image.id === id);
  return index < 0 ? fallback : { ...fallback, gallery: { entries, index } };
}
