/**
 * The key self-check: a **read-only** readout of what the machine actually sends.
 *
 * Why it exists: the panel's keys were designed against what a browser *should* send,
 * and the one thing a design cannot establish is what a given remote control actually
 * sends. The hardware matrix in `VIBE-REMOTE-KEYS.md` is therefore empty until somebody
 * presses the buttons, and this is the instrument they press them at: it prints
 * `key`, `code` and the modifier state for the last presses, so the values can be
 * copied into the table instead of guessed at.
 *
 * Three rules, and all of them are about staying out of the way:
 *
 * - **It is not a command.** No `CommandId`, no handler, nothing in the command table —
 *   so `dispatch` cannot see it and the layer it runs in is unchanged.
 * - **It never consumes a press.** The listener is added in the bubble phase without
 *   `capture`, and it calls neither `preventDefault` nor `stopPropagation`. Every key it
 *   shows still reaches the app exactly as it would have.
 * - **It is off unless asked for.** `import.meta.env.DEV` or an explicit opt-in
 *   (`localStorage["future.vibe.keyProbe"] = "1"`), so a shipped build behaves as if
 *   this file did not exist.
 */

/** The gesture that shows the readout: three quick `⌥⇧K`s, a chord nothing binds. */
export const KEY_PROBE_GESTURE = "alt+shift+k";
/** How many presses of it, and how close together. */
export const KEY_PROBE_TAPS = 3;
export const KEY_PROBE_WINDOW_MS = 1500;

/** What the readout prints for one press. */
export interface ProbeKey {
  key: string;
  code: string;
  meta: boolean;
  ctrl: boolean;
  alt: boolean;
  shift: boolean;
  /** The OS repeating while the key is held — worth seeing for a remote's arrows. */
  repeat: boolean;
}

/** The opt-in flag a real (packaged) build is switched on with. */
export const KEY_PROBE_STORAGE_KEY = "future.vibe.keyProbe";

/**
 * Whether this build offers the readout at all.
 *
 * A developer build always does; a packaged one only after the reader asks for it once
 * (`localStorage.setItem("future.vibe.keyProbe", "1")`), which is how somebody on a sofa
 * with a real remote gets the instrument without the app shipping a debug surface.
 */
export function keyProbeAvailable(
  dev: boolean,
  read: (key: string) => string | null = key => readStorage(key),
): boolean {
  if (dev)
    return true;
  try {
    return read(KEY_PROBE_STORAGE_KEY) === "1";
  }
  catch {
    // Storage can be unavailable (a locked-down webview); "not available" is the answer
    // that changes nothing.
    return false;
  }
}

function readStorage(key: string): string | null {
  try {
    return localStorage.getItem(key);
  }
  catch {
    return null;
  }
}

/** Whether a press is the gesture's chord (and not a repeat of it). */
export function isGesturePress(event: ProbeKey): boolean {
  return !event.repeat
    && event.alt
    && event.shift
    && !event.meta
    && !event.ctrl
    && event.code === "KeyK";
}

/**
 * Fold one press into the tap counter.
 *
 * Kept as a pure step so the gesture is testable without a timer: a press of the chord
 * within `KEY_PROBE_WINDOW_MS` of the previous one counts, anything else starts over.
 */
export function nextTap(
  state: { at: number; taps: number },
  press: { at: number; gesture: boolean },
): { at: number; taps: number; fired: boolean } {
  if (!press.gesture)
    return { at: state.at, taps: 0, fired: false };
  const withinWindow = press.at - state.at <= KEY_PROBE_WINDOW_MS;
  const taps = withinWindow ? state.taps + 1 : 1;
  if (taps >= KEY_PROBE_TAPS)
    return { at: press.at, taps: 0, fired: true };
  return { at: press.at, taps, fired: false };
}

/** The modifiers as a person reads them: `⇧⌥`, or `—` when none are held. */
export function modifierLabel(event: Pick<ProbeKey, "meta" | "ctrl" | "alt" | "shift">): string {
  const parts = [
    event.meta ? "meta" : "",
    event.ctrl ? "ctrl" : "",
    event.alt ? "alt" : "",
    event.shift ? "shift" : "",
  ].filter(Boolean);
  return parts.length > 0 ? parts.join("+") : "—";
}

/**
 * One press as a line to copy into the matrix: `key=ArrowUp code=ArrowUp mods=—`.
 *
 * `key` and `code` are quoted because the values that matter most are the ones a person
 * cannot type by eye: `"Unidentified"`, an empty string, a single space, a full-width
 * character from an input method.
 */
export function describeKey(event: ProbeKey): string {
  return [
    `key=${JSON.stringify(event.key)}`,
    `code=${JSON.stringify(event.code)}`,
    `mods=${modifierLabel(event)}`,
    ...(event.repeat ? ["repeat"] : []),
  ].join(" ");
}
