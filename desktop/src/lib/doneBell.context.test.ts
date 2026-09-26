// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

/**
 * Minimal WebAudio double: the bell must drive a real oscillator/gain graph and
 * must never throw when the API is missing or refuses to start.
 *
 * `doneBell` caches its AudioContext in module scope (one context per session),
 * so every test re-imports the module through `vi.resetModules()` to control
 * whether it starts cold or already warm.
 */
function fakeContext() {
  const frequencies: number[] = [];
  const starts: number[] = [];
  const stops: number[] = [];
  const ramps: { at: number; value: number }[] = [];
  const connected: unknown[] = [];
  const osc = {
    connect: (node: unknown) => {
      connected.push(node);
      return node;
    },
    frequency: {
      set value(value: number) {
        frequencies.push(value);
      },
      get value() {
        return frequencies[frequencies.length - 1] ?? 0;
      },
    },
    start: (at: number) => starts.push(at),
    stop: (at: number) => stops.push(at),
    type: "",
  };
  const gain = {
    connect: (node: unknown) => {
      connected.push(node);
      return node;
    },
    gain: {
      exponentialRampToValueAtTime: (value: number, at: number) => ramps.push({ at, value }),
      setValueAtTime: vi.fn(),
    },
  };
  return {
    ctx: {
      createGain: () => gain,
      createOscillator: () => osc,
      currentTime: 5,
      destination: { kind: "destination" },
    },
    frequencies,
    ramps,
    starts,
    stops,
  };
}

let instantiations = 0;

const originalAudioContext = (window as unknown as { AudioContext?: unknown }).AudioContext;
const originalWebkit = (window as unknown as { webkitAudioContext?: unknown }).webkitAudioContext;

beforeEach(() => {
  instantiations = 0;
});

afterEach(() => {
  vi.restoreAllMocks();
  vi.resetModules();
  (window as unknown as { AudioContext?: unknown }).AudioContext = originalAudioContext;
  (window as unknown as { webkitAudioContext?: unknown }).webkitAudioContext = originalWebkit;
});

function install(behaviour: "resolve" | "throw" = "resolve") {
  const fake = fakeContext();
  class FakeAudioContext {
    constructor() {
      instantiations += 1;
      if (behaviour === "throw")
        throw new Error("AudioContext refused to start");
    }

    createGain = () => fake.ctx.createGain();
    createOscillator = () => fake.ctx.createOscillator();
    currentTime = fake.ctx.currentTime;
    destination = fake.ctx.destination;
  }
  (window as unknown as { AudioContext?: unknown }).AudioContext = FakeAudioContext;
  return fake;
}

/** A cold import of the module, so the context cache starts empty. */
async function freshBell() {
  vi.resetModules();
  return import("./doneBell");
}

describe("playDoneBell", () => {
  it("plays E6 then C7 with exponential fades on a real oscillator graph", async () => {
    const fake = install();
    const { playDoneBell } = await freshBell();

    playDoneBell();

    expect(fake.frequencies).toEqual([1318.51, 2093]);
    expect(fake.starts).toHaveLength(2);
    // The second note starts later, so the pair reads as a ding-dong.
    expect(fake.starts[1]!).toBeGreaterThan(fake.starts[0]!);
    // Both notes stop, after their own duration plus the release margin.
    expect(fake.stops).toHaveLength(2);
    expect(fake.stops[0]!).toBeGreaterThan(fake.starts[0]!);
    expect(fake.stops[1]!).toBeGreaterThan(fake.stops[0]!);
    // Two ramps per note: a fast attack and a long decay, both exponential
    // (a linear attack would click).
    expect(fake.ramps).toHaveLength(4);
    expect(fake.ramps.map(ramp => ramp.value)).toEqual([0.22, 0.0001, 0.16, 0.0001]);
    expect(instantiations).toBe(1);
  });

  it("reuses one AudioContext across bells instead of leaking a context per run", async () => {
    const fake = install();
    const { playDoneBell } = await freshBell();

    playDoneBell();
    playDoneBell();
    playDoneBell();

    expect(instantiations).toBe(1);
    expect(fake.frequencies).toHaveLength(6);
  });

  it("stays silent when the platform has no WebAudio at all", async () => {
    (window as unknown as { AudioContext?: unknown }).AudioContext = undefined;
    (window as unknown as { webkitAudioContext?: unknown }).webkitAudioContext = undefined;
    const { playDoneBell } = await freshBell();

    expect(() => playDoneBell()).not.toThrow();
    expect(instantiations).toBe(0);
  });

  it("falls back to the webkit-prefixed constructor", async () => {
    const fake = install();
    const Ctor = (window as unknown as { AudioContext: unknown }).AudioContext;
    (window as unknown as { AudioContext?: unknown }).AudioContext = undefined;
    (window as unknown as { webkitAudioContext?: unknown }).webkitAudioContext = Ctor;
    const { playDoneBell } = await freshBell();

    playDoneBell();

    expect(instantiations).toBe(1);
    expect(fake.frequencies).toEqual([1318.51, 2093]);
  });

  it("swallows a constructor that refuses to start (muted system, no device)", async () => {
    install("throw");
    const { playDoneBell } = await freshBell();

    expect(() => playDoneBell()).not.toThrow();
    expect(instantiations).toBe(1);
  });

  it("swallows a failure while building the tone graph", async () => {
    class BrokenContext {
      constructor() {
        instantiations += 1;
      }

      createOscillator = () => {
        throw new Error("no audio device");
      };
    }
    (window as unknown as { AudioContext?: unknown }).AudioContext = BrokenContext;
    const { playDoneBell } = await freshBell();

    expect(() => playDoneBell()).not.toThrow();
    expect(instantiations).toBe(1);
  });

  it("does not cache a context that failed to start, so a later bell can still ring", async () => {
    install("throw");
    const { playDoneBell } = await freshBell();
    playDoneBell();
    expect(instantiations).toBe(1);

    // The failure must not be memoised: a second attempt constructs again.
    playDoneBell();
    expect(instantiations).toBe(2);
  });
});
