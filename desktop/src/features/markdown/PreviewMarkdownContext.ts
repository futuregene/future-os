import { createContext, use } from "react";

/** A file a previewed document links to, already resolved to an absolute path. */
export interface PreviewFileTarget {
  name: string;
  path: string;
}

/**
 * Marks a `MarkdownContent` subtree as rendering inside the local-file preview
 * (the fullscreen `MarkdownPreview` overlay or the artifact detail preview),
 * rather than the chat message stream. Presence of this context is the
 * "preview mode" signal the link renderers branch on:
 *  - local-file links follow into a nested preview when the host can open one
 *    (`openPreview`), and otherwise open with the OS handler,
 *  - no custom right-click menu,
 *  - relative link targets resolve against `basePath`'s directory rather than
 *    the workspace root.
 *
 * The chat stream never provides this context, so its behavior is unchanged.
 */
export interface PreviewMarkdownContextValue {
  /** Absolute path of the file being previewed; base for resolving relative links. */
  basePath: string;
  /**
   * Open a previewable local file one level deeper, in place of the current
   * document. Provided by the fullscreen overlay, whose stack of levels the
   * reader walks back up with Escape; absent when the preview has nowhere to
   * nest (the artifact detail's inline preview), where the OS handler stands in.
   */
  openPreview?: (file: PreviewFileTarget) => void;
}

export const PreviewMarkdownContext = createContext<PreviewMarkdownContextValue | null>(null);

export function usePreviewMarkdown() {
  return use(PreviewMarkdownContext);
}
