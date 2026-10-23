import { extendTailwindMerge } from "tailwind-merge";

type ClassValue = string | false | null | undefined;

/**
 * Merge, taught about the type steps this app adds for itself.
 *
 * `tailwind-merge` decides conflicts from its own table of Tailwind's classes.
 * It knows Tailwind's sizes (`text-sm`, and even `text-xs` / `text-2xs`), but a
 * size declared in `globals.css` — `--text-13` → `text-13` — is an unknown
 * `text-*` to it, and an unknown `text-*` is read as a text **colour**. So
 * `cn("text-13 … text-ink-soft")` kept the colour and silently dropped the size:
 * the element rendered at the inherited size and nothing anywhere failed. The
 * rail's rows hit exactly that when their titles moved to `text-13`.
 *
 * Declaring it as a size is what makes the token usable inside `cn()`, which is
 * most of the app. A new `--text-<n>` step needs a line here too.
 */
const merge = extendTailwindMerge({
  extend: { classGroups: { "font-size": [{ text: ["13"] }] } },
});

/**
 * Join class names and resolve Tailwind conflicts (last wins), so a caller's
 * `className` reliably overrides a component's defaults instead of depending on
 * CSS source order.
 */
export function cn(...values: ClassValue[]) {
  return merge(values.filter(Boolean).join(" "));
}
