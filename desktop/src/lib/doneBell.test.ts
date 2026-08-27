// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from "vitest";

/**
 * Fake AudioContext: records oscillator/gain wiring so tests can assert the
 * two-note shape without any real audio device.
 */
class FakeOscillator {
  type = "";
  frequency = { value: 0 };
  connect = vi.fn().mockReturnThis();
  start = vi.fn();
  stop = vi.fn();
}
class FakeGain {
  gain = {
    value: 0,
    setValueAtTime: vi.fn(),
    exponentialRampToValueAtTime: vi.fn(),
  };

  connect = vi.fn().mockReturnThis();
}
class FakeAudioContext {
  currentTime = 0;
  destination = {};
  createOscillator = vi.fn(() => new FakeOscillator());
  createGain = vi.fn(() => new FakeGain());
}

describe("playDoneBell", () => {
  let audioContext: FakeAudioContext;

  beforeEach(() => {
    audioContext = new FakeAudioContext();
    // A constructible class whose constructor returns the fake instance —
    // an arrow function can't be `new`-ed by the production code.
    const StubAudioContext = class {
      constructor() {
        return audioContext as unknown as this;
      }
    };
    vi.stubGlobal("AudioContext", StubAudioContext);
    vi.resetModules();
  });

  it("synthesizes the two-note ding without any asset", async () => {
    const { playDoneBell: play } = await import("./doneBell");
    play();
    expect(audioContext.createOscillator).toHaveBeenCalledTimes(2);
    expect(audioContext.createGain).toHaveBeenCalledTimes(2);
    const first = audioContext.createOscillator.mock.results[0];
    expect(first?.value.frequency.value).toBe(1318.51); // E6 → C7 ding-dong
  });

  it("is a silent no-op when AudioContext is unavailable", async () => {
    vi.stubGlobal("AudioContext", undefined);
    const { playDoneBell: play } = await import("./doneBell");
    expect(() => play()).not.toThrow();
  });

  it("never throws when the audio graph fails mid-way", async () => {
    audioContext.createOscillator.mockImplementation(() => {
      throw new Error("no audio device");
    });
    const { playDoneBell: play } = await import("./doneBell");
    expect(() => play()).not.toThrow();
  });
});
