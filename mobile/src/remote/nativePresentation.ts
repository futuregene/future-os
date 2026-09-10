/**
 * Native surfaces presented *over* the app — the system camera, the photo
 * picker, the document picker — are separate Android activities / iOS view
 * controllers. Presenting one pauses the host activity, which React Native
 * reports as AppState `background` even though the user never left the flow
 * they started here.
 *
 * `RemoteClient.setAppActive(false)` deliberately tears the WSS down on
 * background. For a photo round trip that is the wrong call: it flashes
 * "the desktop may be sleeping" on the way back and spends a whole handshake
 * to restore a link that was never actually lost. So while a picker is on
 * screen the app is still treated as active; the caller in
 * `useRemoteConnection` bounds how long that can last, so a picker the user
 * never returns from still releases the socket.
 */
let presentations = 0;

/**
 * How long the OS-reported background may be ignored while a picker is on
 * screen. A camera round trip is seconds; anything past this is a flow the
 * user walked away from, and the connection should be released.
 */
export const NATIVE_PRESENTATION_GRACE_MS = 60_000;

/** A native picker is about to be presented. */
export function beginNativePresentation(): void {
  presentations += 1;
}

/** Its result (or cancellation) has been delivered. */
export function endNativePresentation(): void {
  presentations = Math.max(0, presentations - 1);
}

export function nativePresentationInFlight(): boolean {
  return presentations > 0;
}

/** Run a native presentation with the app treated as active throughout. */
export async function withNativePresentation<T>(launch: () => Promise<T>): Promise<T> {
  beginNativePresentation();
  try {
    return await launch();
  } finally {
    endNativePresentation();
  }
}
