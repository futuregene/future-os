/**
 * Completion bell synthesized with the WebAudio API — no audio asset, works
 * identically on macOS / Windows / Linux. Two sine notes (E6 → C7) give a
 * short "ding-dong" that is unmistakable but not harsh.
 *
 * Deliberately silent-and-safe on failure: jsdom tests, muted systems, or
 * browsers without an AudioContext must never throw or block the UI thread.
 */

let ctx: AudioContext | null = null;

const NOTE_E6 = 1318.51;
const NOTE_C7 = 2093.0;

function bellContext(): AudioContext | null {
  if (ctx)
    return ctx;
  const Ctor = window.AudioContext ?? (window as unknown as { webkitAudioContext?: typeof AudioContext }).webkitAudioContext;
  if (!Ctor)
    return null;
  ctx = new Ctor();
  return ctx;
}

function tone(ac: AudioContext, start: number, frequency: number, duration: number, volume: number) {
  const osc = ac.createOscillator();
  const gain = ac.createGain();
  osc.type = "sine";
  osc.frequency.value = frequency;
  // Exponential ramps cannot start from 0 — climb from a tiny value.
  gain.gain.setValueAtTime(0.0001, start);
  gain.gain.exponentialRampToValueAtTime(volume, start + 0.015);
  gain.gain.exponentialRampToValueAtTime(0.0001, start + duration);
  osc.connect(gain).connect(ac.destination);
  osc.start(start);
  osc.stop(start + duration + 0.02);
}

export function playDoneBell() {
  try {
    const ac = bellContext();
    if (!ac)
      return;
    const now = ac.currentTime;
    tone(ac, now, NOTE_E6, 0.16, 0.22);
    tone(ac, now + 0.14, NOTE_C7, 0.22, 0.16);
  }
  catch {
    // Audio unavailable — the attention request (caller side) still fires.
  }
}
