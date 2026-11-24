import { createContext, use } from "react";

/**
 * One picture in a rendered Markdown document, as the lightbox needs it: a URL
 * the browser can already draw (`src` is resolved by `MarkdownImage` before this
 * is registered) and the words that name it.
 */
export interface GalleryImage {
  alt: string;
  /** Stable per rendered image (`useId`), so stepping can find it by identity. */
  id: string;
  src: string;
  title?: string;
}

/**
 * The pictures one Markdown document drew, in the order it drew them.
 *
 * Deliberately not React state: images register from effects once their own
 * bytes resolve, and a state update per picture would re-render the whole
 * transcript on every load. The lightbox takes a snapshot when it opens, which
 * is also the only moment the list has to be accurate.
 */
export interface ImageGallery {
  add: (image: GalleryImage) => void;
  remove: (id: string) => void;
  snapshot: () => GalleryImage[];
}

export function createImageGallery(): ImageGallery {
  // A Map keeps insertion order, which is mount order — the document's own order.
  const images = new Map<string, GalleryImage>();
  return {
    add(image) {
      images.set(image.id, image);
    },
    remove(id) {
      images.delete(id);
    },
    snapshot() {
      return [...images.values()];
    },
  };
}

/**
 * Scoped to one `MarkdownContent`, so "the images in this drawing" means the
 * pictures of this reply rather than every picture in the conversation.
 */
export const ImageGalleryContext = createContext<ImageGallery | null>(null);

export function useImageGallery(): ImageGallery | null {
  return use(ImageGalleryContext);
}
