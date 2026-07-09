import { createContext, use } from "react";

/**
 * Marks a `MarkdownContent` subtree as rendering inside the local-file preview
 * (the fullscreen `MarkdownPreview` overlay or the artifact detail preview),
 * rather than the chat message stream. Presence of this context is the
 * "preview mode" signal the link renderers branch on:
 *  - local-file links open with the OS handler instead of an in-app preview,
 *  - no custom right-click menu,
 *  - relative link targets resolve against `basePath`'s directory rather than
 *    the workspace root.
 *
 * The chat stream never provides this context, so its behavior is unchanged.
 */
export interface PreviewMarkdownContextValue {
  /** Absolute path of the file being previewed; base for resolving relative links. */
  basePath: string;
}

export const PreviewMarkdownContext = createContext<PreviewMarkdownContextValue | null>(null);

export function usePreviewMarkdown() {
  return use(PreviewMarkdownContext);
}
