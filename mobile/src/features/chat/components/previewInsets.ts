import { Platform } from "react-native";
import type { EdgeInsets } from "react-native-safe-area-context";

/**
 * Insets for a full-screen reader's own surface, read from the app's
 * `SafeAreaProvider` context rather than from a `SafeAreaView`.
 *
 * The reader covers the status bar on purpose (`statusBarTranslucent` where it
 * uses it): the system keeps drawing its status bar over the top of the window,
 * so the header has to be pushed back out of the bar's reach. A `SafeAreaView`
 * cannot do that here — a Modal is its own native window, so nothing inside it
 * is a native descendant of the app's provider, which is the only thing a
 * `SafeAreaView` resolves insets through. It therefore applied nothing: the
 * header sat at the top of the screen, and the status bar's hit area swallowed
 * the taps aimed at its buttons (measured on the reporting device: the header
 * occupied rows 0–60dp, its buttons' 44-point hit boxes rows 8–52dp, inside the
 * bar). The context insets describe the window the app is drawn in, which is
 * where that status bar is.
 *
 * Only the top edge is ours: `statusBarTranslucent` extends the reader under the
 * status bar alone, while the other edges stay laid out by the window (and the
 * reader's body is full-bleed by design).
 *
 * iOS takes nothing: its reader is a `pageSheet`, which the system already
 * places below the status bar, so the context's window inset for the top edge
 * would only open a gap above the header.
 *
 * This padding only moves the document if it lands on an ancestor that lays its
 * children out *normally*: Yoga positions an `absoluteFill` child from its
 * parent's border edge and sizes it to the border box, so a parent's padding
 * never reaches one (verified against Yoga 3.2.1: an absolute child of a box
 * padded by 30 kept `top = 0` and its full height). The file reader's layers are
 * exactly such children, which is why they hang off a `flex: 1` child that the
 * padding does move rather than off the padded box directly.
 */
export function previewSurfaceInsets(insets: EdgeInsets) {
  if (Platform.OS !== "android") return null;
  return { paddingTop: insets.top };
}
