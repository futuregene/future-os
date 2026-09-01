/**
 * A small in-process serial lane for storage operations that must preserve
 * call order across multiple awaits. A rejected operation never poisons the
 * lane: later work still runs after it settles.
 */
export function createAsyncOperationQueue() {
  let tail: Promise<void> = Promise.resolve();

  return function enqueue<T>(operation: () => Promise<T>): Promise<T> {
    const result = tail.then(operation, operation);
    tail = result.then(
      () => undefined,
      () => undefined,
    );
    return result;
  };
}
