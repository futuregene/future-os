import { useLayoutEffect, useRef, useState } from "react";

/** The bottom of the nearest scroll/clip ancestor, or the viewport bottom. */
function clippingBottom(element: HTMLElement): number {
  let node = element.parentElement;
  while (node) {
    const overflowY = getComputedStyle(node).overflowY;
    if (overflowY === "auto" || overflowY === "scroll" || overflowY === "hidden")
      return node.getBoundingClientRect().bottom;
    node = node.parentElement;
  }
  return window.innerHeight;
}

/**
 * Position a dropdown below its trigger by default, flipping above when it would
 * spill past its scrolling container (e.g. the last thread near the sidebar
 * bottom) so the whole menu — including Delete — stays visible. Attach the
 * returned `menuRef` to the menu element and use `dropUp` to pick the offset.
 *
 * `open` gates the measurement for menus that mount before they're shown; pass
 * `true` (the default) for menus that only render while open.
 */
export function useDropUpMenu(open: boolean = true) {
  const menuRef = useRef<HTMLDivElement>(null);
  const [dropUp, setDropUp] = useState(false);
  useLayoutEffect(() => {
    if (!open)
      return;
    const element = menuRef.current;
    if (!element)
      return;
    const rect = element.getBoundingClientRect();
    const boundary = Math.min(clippingBottom(element), window.innerHeight) - 8;
    setDropUp(rect.bottom > boundary);
  }, [open]);
  return { menuRef, dropUp };
}
