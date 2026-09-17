/**
 * Web stand-in for the `future-share-intent` native module.
 *
 * Reports a payload only when the page is opened with `?share=1`, so the share
 * intake sheet appears in the scenarios that are about sharing and nowhere else
 * (otherwise every other capture would start behind a modal).
 * See docs/guide/screenshots.md.
 */
export interface SharedFile {
  uri: string;
  name: string;
  mimeType: string;
}

export interface SharedContent {
  text: string;
  files: SharedFile[];
  tooLarge: boolean;
  failed?: boolean;
}

function shareRequested(): boolean {
  try {
    return new URLSearchParams(location.search).has("share");
  }
  catch {
    return false;
  }
}

export function addPendingShareListener(_listener: () => void): { remove(): void } {
  return { remove() {} };
}

export async function getPendingShare(): Promise<SharedContent | null> {
  if (!shareRequested())
    return null;
  return {
    text: "这篇刚出的预印本，帮我看一下和我们的结论有没有冲突。",
    files: [
      { uri: "http://127.0.0.1:7392/effect-size.png", name: "preprint-2026-0917.pdf", mimeType: "application/pdf" },
      { uri: "http://127.0.0.1:7392/forest-plot.png", name: "IMG_2043.png", mimeType: "image/png" },
    ],
    tooLarge: false,
  };
}
