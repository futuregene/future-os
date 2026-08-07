import { twMerge } from "tailwind-merge";

type ClassValue = string | false | null | undefined;

/**
 * Join class names and resolve Tailwind conflicts (last wins), so a caller's
 * `className` reliably overrides a component's defaults instead of depending on
 * CSS source order.
 */
export function cn(...values: ClassValue[]) {
  return twMerge(values.filter(Boolean).join(" "));
}
