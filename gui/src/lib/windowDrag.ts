import type { MouseEvent } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";

export function startWindowDrag(event: MouseEvent<HTMLElement>) {
  if (event.button !== 0)
    return;

  const target = event.target;
  if (target instanceof Element && target.closest("button, input, textarea, select, a")) {
    return;
  }

  event.preventDefault();
  document.getSelection()?.removeAllRanges();

  void getCurrentWindow().startDragging().catch(() => {});
}
