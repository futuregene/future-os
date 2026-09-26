import { ConnectionGeneration } from "../connectionGeneration";

beforeEach(() => jest.useFakeTimers());
afterEach(() => jest.useRealTimers());

test("activation yields in bounded batches and new events cannot overtake the buffered prefix", async () => {
  const owner = new ConnectionGeneration(1);
  const seen: number[] = [];
  for (let i = 0; i < 500; i++) owner.deliver(() => seen.push(i), 1024);
  owner.activate();
  expect(seen).toHaveLength(64);
  owner.deliver(() => seen.push(500), 1024);
  await jest.runAllTimersAsync();
  expect(seen).toEqual(Array.from({ length: 501 }, (_, i) => i));
  owner.retire();
});

test("retiring a yielding generation fences the queued suffix", async () => {
  const owner = new ConnectionGeneration(1);
  const deliver = jest.fn();
  for (let i = 0; i < 500; i++) owner.deliver(deliver, 1024);
  owner.activate();
  owner.retire();
  await jest.runAllTimersAsync();
  expect(deliver).toHaveBeenCalledTimes(64);
  expect(jest.getTimerCount()).toBe(0);
});

/** The private drain, called directly to reach its post-retirement guards. */
const drainOf = (owner: ConnectionGeneration) => (owner as unknown as { drain(): void }).drain();

test("a retired generation drops late deliveries instead of buffering them", () => {
  const owner = new ConnectionGeneration(1);
  const deliver = jest.fn();
  owner.retire();
  owner.deliver(deliver, 1);
  expect(deliver).not.toHaveBeenCalled();
  expect(owner.live).toBe(false);
  // A drain that was already queued when the generation retired is inert: the
  // buffered callback must not run against a peer that has been let go.
  drainOf(owner);
  expect(deliver).not.toHaveBeenCalled();
});

test("a candidate that overflows its buffer fails rather than growing without bound", () => {
  const owner = new ConnectionGeneration(1);
  for (let index = 0; index < 4096; index += 1) owner.deliver(() => {}, 1);
  expect(owner.live).toBe(true);
  // The 4097th entry cannot be buffered behind the readiness barrier.
  expect(() => owner.deliver(() => {}, 1)).not.toThrow();
  expect(owner.live).toBe(false);
  expect(() => owner.check()).toThrow("candidate_buffer_exhausted");
});

test("an oversized single event fails the candidate even with room in the list", () => {
  const owner = new ConnectionGeneration(1);
  owner.deliver(() => {}, 4 * 1024 * 1024);
  expect(() => owner.deliver(() => {}, 5 * 1024 * 1024)).not.toThrow();
  expect(() => owner.check()).toThrow("candidate_buffer_exhausted");
});

test("a serving generation between batches reports overflow to its caller", () => {
  const owner = new ConnectionGeneration(1);
  // 4095 two-KiB entries stay inside both limits while buffering…
  for (let index = 0; index < 4095; index += 1) owner.deliver(() => {}, 2048);
  owner.activate();
  // …activate() delivers the first 64 and leaves the drain yielding, so the
  // generation is serving-with-a-backlog: the next overflow must be thrown to
  // the caller, which is mid-delivery and can act on it.
  for (let index = 0; index < 65; index += 1) owner.deliver(() => {}, 2048);
  expect(() => owner.deliver(() => {}, 2048)).toThrow("candidate_buffer_exhausted");
});

test("a serving generation delivers straight through once its backlog is gone", () => {
  const owner = new ConnectionGeneration(1);
  owner.activate();
  const deliver = jest.fn();
  owner.deliver(deliver, 1024);
  // No buffering and no timer: the peer is live and the readiness barrier is
  // behind us, so the event goes out on the caller's stack.
  expect(deliver).toHaveBeenCalledTimes(1);
  expect(jest.getTimerCount()).toBe(0);
});

test("a retirement caused by a delivery stops the rest of the batch", () => {
  const owner = new ConnectionGeneration(1);
  const seen: number[] = [];
  for (let index = 0; index < 200; index += 1) {
    owner.deliver(() => {
      seen.push(index);
      if (seen.length === 2) owner.retire();
    }, 1024);
  }
  owner.activate();
  expect(seen).toEqual([0, 1]);
  // Nothing further is scheduled for a generation that retired mid-batch.
  expect(jest.getTimerCount()).toBe(0);
});

