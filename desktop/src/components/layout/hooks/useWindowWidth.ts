import { useEffect, useState } from "react";

/**
 * Live viewport width in CSS pixels, re-read on every window resize.
 *
 * Drives the shell's collapse decisions: below the three-column floors the side
 * panels fold instead of squeezing the conversation out of the window. A plain
 * resize subscription, not cancel-safe loading (desktop/CLAUDE.md §4).
 */
export function useWindowWidth(): number {
  const [width, setWidth] = useState(() => window.innerWidth);

  useEffect(() => {
    const onResize = () => setWidth(window.innerWidth);
    window.addEventListener("resize", onResize);
    return () => window.removeEventListener("resize", onResize);
  }, []);

  return width;
}
