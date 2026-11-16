/**
 * Mobile's test environment: jest-expo's native environment, with the
 * environment's own timers put back after a fake-timer round.
 *
 * `jest.useRealTimers()` reaches `@sinonjs/fake-timers`' `uninstall()` (the call
 * behind it), which decides between restoring and deleting per clocked method:
 *
 *     if (_global[method] && _global[method].hadOwnProperty) restore
 *     else delete _global[method]
 *
 * `hadOwnProperty` is captured as `hasOwnProperty.call(target, method)` when the
 * clock installs. On Node 26 the global Jest hands a test file is a proxy where
 * `setTimeout` and friends are **not** own properties — `hasOwnProperty` and `in`
 * both answer false, while `getOwnPropertyDescriptor` still returns the real
 * descriptor, and `Object.defineProperty` cannot change that answer — so
 * fake-timers always takes its `delete` branch and the timer globals react-native
 * `setup-env` installed disappear instead of coming back.
 *
 * Nothing notices until something unmounts. `VirtualizedList` calls
 * `clearTimeout` on unmount, so any test that tears a list down after a
 * fake-timer round dies with `ReferenceError: clearTimeout is not defined` — a
 * failure that has nothing to do with what it asserts. Measured on
 * `dev-huichen` (2026-11-15): on Node 26 that is 13 suites / 298 tests, the
 * `MarkdownText.test.ts` soft-break case among them; Node 22 and Node 24
 * (`.nvmrc`, CI) keep the own property and those suites are green there.
 *
 * `jest` is one object per module, so no setup file can wrap `useRealTimers`
 * for the tests — but every module's `jest.useRealTimers()` goes through this
 * file's `environment.fakeTimers*` instance, so the restore belongs here. A
 * no-op on Node 24: the own property survives, so nothing is missing.
 */
const { TestEnvironment: ExpoEnvironment } = require("jest-expo/src/preset/nativeEnvironment");

/** Clocked methods the environment supplies before any test replaces them. */
const CLOCKED = [
  "setTimeout", "clearTimeout", "setInterval", "clearInterval",
  "setImmediate", "clearImmediate", "queueMicrotask",
  "requestAnimationFrame", "cancelAnimationFrame",
  "requestIdleCallback", "cancelIdleCallback",
];

/**
 * Snapshot the timers just before the clock replaces them, and put back any it
 * removed afterwards. A `useFakeTimers` that throws (already faking) is not ours
 * to handle: the clock's own error is the useful one.
 */
function keepEnvironmentTimers(fakeTimers, globalObject) {
  if (!fakeTimers) return;
  let snapshot = null;
  const capture = () => {
    if (snapshot) return;
    snapshot = CLOCKED
      .map(name => [name, globalObject[name]])
      .filter(([, value]) => typeof value === "function");
  };
  const restore = () => {
    for (const [name, value] of snapshot ?? []) {
      if (typeof globalObject[name] === "function") continue;
      // `defineProperty`, not assignment: these are accessors on a proxy, where
      // a plain set does not always land.
      Object.defineProperty(globalObject, name, {
        value, writable: true, configurable: true, enumerable: true,
      });
    }
  };

  const clockOn = fakeTimers.useFakeTimers.bind(fakeTimers);
  fakeTimers.useFakeTimers = config => {
    capture();
    clockOn(config);
  };
  const clockOff = fakeTimers.useRealTimers.bind(fakeTimers);
  fakeTimers.useRealTimers = () => {
    clockOff();
    restore();
  };
}

class FutureTestEnvironment extends ExpoEnvironment {
  constructor(config, context) {
    super(config, context);
    // Legacy and modern clocks are separate instances; only one is used per run.
    keepEnvironmentTimers(this.fakeTimers, this.global);
    keepEnvironmentTimers(this.fakeTimersModern, this.global);
  }
}

module.exports = FutureTestEnvironment;
module.exports.TestEnvironment = FutureTestEnvironment;
