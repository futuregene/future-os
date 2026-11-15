/**
 * The one contract `jest.environment.js` exists for: a fake-timer round must
 * hand the environment's own timers back.
 *
 * `@sinonjs/fake-timers` (behind `jest.useRealTimers`) restores a clocked
 * method only when it was an own property of the global at install time, and
 * *deletes* it otherwise. On Node 26 the global is a proxy where `setTimeout`
 * and friends are not own properties, so the timers react-native `setup-env`
 * installed vanish, and the next unmount of a `VirtualizedList` — which calls
 * `clearTimeout` — throws `ReferenceError: clearTimeout is not defined` in
 * whatever test happens to tear it down (measured: 13 suites / 298 tests on
 * `dev-huichen`, 2026-11-15). This pins the environment's side of that.
 *
 * It is dormant on Node 24 (`.nvmrc`, CI): there the timers were never lost, so
 * removing the restore from `jest.environment.js` leaves this test — and CI —
 * green. The canary for that regression is the mobile suites that unmount a
 * list after a fake-timer round; run them on Node 26.
 */
const CLOCKED = ["setTimeout", "clearTimeout", "setInterval", "clearInterval", "setImmediate", "clearImmediate"] as const;

test("a fake-timer round gives the environment its timers back", () => {
  const environmentTimers = CLOCKED.map(name => [name, globalThis[name]] as const);
  // Only a snapshot of real functions can prove anything about what came back.
  for (const [name, value] of environmentTimers) expect([name, typeof value]).toEqual([name, "function"]);

  jest.useFakeTimers();
  jest.useRealTimers();

  // The same functions, not lookalikes: code that captured one before the round
  // keeps clearing against the id space it started with.
  for (const [name, value] of environmentTimers) expect([name, globalThis[name]]).toEqual([name, value]);
});
