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
