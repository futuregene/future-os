import { useEffect, useState } from "react";
import { Keyboard, Platform } from "react-native";

/**
 * The on-screen keyboard's height, or 0 while it is closed.
 *
 * Android runs this app edge-to-edge, where `adjustResize` no longer shrinks
 * the window and React Native's `KeyboardAvoidingView` measures the keyboard
 * from the wrong origin (see `ChatScreen`). A screen that scrolls must reserve
 * the same space itself, or its last rows — a form's Save button, the bottom of
 * a long settings page — sit underneath the keyboard with no way to scroll to
 * them.
 *
 * Only Android needs the number: iOS still gets its inset from
 * `KeyboardAvoidingView`'s padding, so this stays 0 there and the two do not
 * double-count.
 */
export function useKeyboardHeight(): number {
  const [height, setHeight] = useState(0);
  useEffect(() => {
    if (Platform.OS !== "android") return;
    const show = Keyboard.addListener("keyboardDidShow", event =>
      setHeight(event.endCoordinates.height),
    );
    const hide = Keyboard.addListener("keyboardDidHide", () => setHeight(0));
    return () => { show.remove(); hide.remove(); };
  }, []);
  return height;
}
