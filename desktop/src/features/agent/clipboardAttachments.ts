import { localFilePath } from "@future-os/markdown";

/**
 * Turn the browser-standard `text/uri-list` clipboard payload into local paths.
 * Comments and non-file URIs are intentionally ignored: only file URIs express
 * a filesystem reference that the desktop backend can inspect directly.
 */
export function localPathsFromUriList(uriList: string): string[] {
  const paths = new Set<string>();
  for (const value of uriList.split(/\r?\n/)) {
    const uri = value.trim();
    if (!uri || uri.startsWith("#") || !/^file:/i.test(uri))
      continue;
    const path = localFilePath(uri);
    if (path)
      paths.add(path);
  }
  return [...paths];
}
