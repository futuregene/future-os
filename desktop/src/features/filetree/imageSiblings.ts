import type { DirEntry } from "../../integrations/storage/files";
import { previewKindForPath } from "../filepreview/previewKind";

/**
 * Directory that holds `path`, using whichever separator the platform wrote
 * (`\` on Windows). Returns "" when the path has no directory part.
 */
export function parentDirectory(path: string): string {
  const cut = Math.max(path.lastIndexOf("/"), path.lastIndexOf("\\"));
  return cut <= 0 ? "" : path.slice(0, cut);
}

/**
 * Images that sit next to `path` in one directory listing, in the listing's own
 * order — the order the file tree draws its rows in, so "next" always means the
 * row below the one that was opened.
 *
 * The preview's ← / → walk this list, which is what makes a folder of figures
 * browsable without a round trip through the tree for each one. The list
 * includes `path` itself (the caller finds its position); when `path` is not in
 * `entries` — a stale listing, or a file opened from somewhere the tree does not
 * hold — the result is empty and the preview offers no navigation rather than
 * jumping into a list that does not contain what is on screen.
 */
export function siblingImages(entries: DirEntry[] | null | undefined, path: string): DirEntry[] {
  if (!entries)
    return [];
  const images = entries.filter(entry => !entry.isDir && previewKindForPath(entry.path) === "image");
  return images.some(entry => entry.path === path) ? images : [];
}
