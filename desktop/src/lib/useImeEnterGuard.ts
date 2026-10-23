import { useRef } from "react";

function isEnter(event: Pick<KeyboardEvent, "key" | "code">): boolean {
  return event.key === "Enter" || event.code === "Enter" || event.code === "NumpadEnter";
}

/** Keep the Enter that confirms an IME candidate owned by the IME until release. */
export function useImeEnterGuard() {
  const isComposingRef = useRef(false);
  const imeEnterHeldRef = useRef(false);

  function shouldIgnoreKeyDown(event: KeyboardEvent): boolean {
    const enter = isEnter(event);
    // Some webviews end composition before the committing keydown, reporting
    // only keyCode 229. Do not cancel that event: the IME still needs it.
    if (isComposingRef.current || event.isComposing || event.keyCode === 229 || event.key === "Unidentified") {
      if (enter)
        imeEnterHeldRef.current = true;
      return true;
    }
    // Composition may already have ended while the physical Enter is held.
    // Ignore repeats even if the webview never delivered the first keydown.
    if (enter && (imeEnterHeldRef.current || event.repeat)) {
      event.preventDefault();
      return true;
    }
    return false;
  }

  function onKeyUp(event: Pick<KeyboardEvent, "key" | "code">) {
    if (isEnter(event))
      imeEnterHeldRef.current = false;
  }

  function onCompositionStart() {
    isComposingRef.current = true;
  }

  function onCompositionEnd() {
    isComposingRef.current = false;
  }

  function onBlur() {
    // A release outside the field may never reach it. Do not leave the next
    // visit stuck in a composition or waiting for an already-released Enter.
    isComposingRef.current = false;
    imeEnterHeldRef.current = false;
  }

  return { isComposingRef, onBlur, onCompositionEnd, onCompositionStart, onKeyUp, shouldIgnoreKeyDown };
}
