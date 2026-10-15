import { createContext } from "react";

/**
 * Which surface a markdown body is drawn on.
 *
 * `default` is a message on the app's own surface. `onAccent` is the user's own
 * bubble, which is filled with the accent colour: every colour the markdown
 * sheet paints has to be read against *that* fill, because the palette's text
 * and recess colours are drawn for `surface` and vanish into the accent.
 *
 * Its own module because the sheet and the pieces beside it (a formula's tint)
 * both read it, and the formula is rendered from inside the sheet.
 */
export type MarkdownTone = "default" | "onAccent";

export const MarkdownToneContext = createContext<MarkdownTone>("default");
