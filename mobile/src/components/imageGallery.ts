import { createContext, useContext } from "react";
import { remoteMarkdownImageUrl } from "@future-os/markdown";

/**
 * One picture in a rendered reply, as the full-screen viewer needs it.
 *
 * `uri` is the bytes this phone already holds, when it has them. `src` plus
 * `basePath` is the fallback: a picture that is still loading (or whose reply
 * was opened from a document) is resolved by the viewer through the same loader
 * the transcript uses, so stepping never shows an empty frame for a picture the
 * reader can already see in the reply.
 */
export interface GalleryImage {
  /** Resolves a relative `src` against the document that contains it. */
  basePath?: string;
  /** Stable per rendered image (`useId`); registration order is document order. */
  id: string;
  name: string;
  src: string;
  uri: string | null;
}

/**
 * The pictures one reply drew, in the order it drew them.
 *
 * A plain store rather than React state: each picture registers once it is
 * mounted, and an update per load would re-render the whole transcript. The
 * viewer takes a snapshot when it opens, and the entries carry enough to finish
 * resolving any picture that was not loaded yet.
 */
export interface ImageGallery {
  remove(id: string): void;
  snapshot(): GalleryImage[];
  upsert(image: GalleryImage): void;
}

export function createImageGallery(): ImageGallery {
  // A Map keeps insertion order. Updating an existing id keeps its place, so a
  // picture resolving late does not jump to the end of the sequence.
  const images = new Map<string, GalleryImage>();
  return {
    remove(id) {
      images.delete(id);
    },
    snapshot() {
      return [...images.values()];
    },
    upsert(image) {
      images.set(image.id, image);
    },
  };
}

/** The pictures a reply's viewer may page through; absent on other surfaces. */
export const ImageGalleryContext = createContext<ImageGallery | null>(null);

export function useImageGallery(): ImageGallery | null {
  return useContext(ImageGalleryContext);
}

/** The bytes a gallery entry draws, when it can be known without a load. */
export function immediateGalleryUri(image: GalleryImage): string | null {
  return image.uri ?? remoteMarkdownImageUrl(image.src);
}
