import { remoteHttp } from "../http";

describe("remote HTTP cancellation", () => {
  const originalFetch = globalThis.fetch;
  beforeEach(() => {
    jest.useFakeTimers();
  });
  afterEach(() => {
    globalThis.fetch = originalFetch;
    jest.useRealTimers();
  });

  function pendingFetch() {
    const fetch = jest.fn(
      (_url: string, options: RequestInit) =>
        new Promise<Response>((_resolve, reject) => {
          options.signal!.addEventListener("abort", () => reject(new Error("aborted")), {
            once: true,
          });
        }),
    );
    globalThis.fetch = fetch as typeof globalThis.fetch;
    return fetch;
  }

  test("deadline aborts the underlying request", async () => {
    const fetch = pendingFetch();
    const pending = remoteHttp("https://example.test", {}, (response) => response.json());
    const failure = expect(pending).rejects.toThrow("network_request_timeout");
    await jest.advanceTimersByTimeAsync(20_000);
    await failure;
    expect(fetch.mock.calls[0]![1].signal!.aborted).toBe(true);
    expect(jest.getTimerCount()).toBe(0);
  });

  test("caller cancellation aborts immediately and releases its deadline", async () => {
    pendingFetch();
    const controller = new AbortController();
    const pending = remoteHttp(
      "https://example.test",
      {},
      (response) => response.json(),
      controller.signal,
    );
    const failure = expect(pending).rejects.toThrow("remote_request_cancelled");
    controller.abort();
    await failure;
    expect(jest.getTimerCount()).toBe(0);
  });

  test("deadline also bounds a stalled response body", async () => {
    globalThis.fetch = jest.fn(async () => ({
      json: () => new Promise(() => {}),
    })) as unknown as typeof fetch;
    const pending = remoteHttp("https://example.test", {}, (response) => response.json());
    const failure = expect(pending).rejects.toThrow("network_request_timeout");
    await jest.advanceTimersByTimeAsync(20_000);
    await failure;
    expect(jest.getTimerCount()).toBe(0);
  });

  test("a signal already aborted before the call never reaches the network", async () => {
    jest.useFakeTimers();
    const fetchMock = jest.fn();
    globalThis.fetch = fetchMock as unknown as typeof fetch;
    const controller = new AbortController();
    controller.abort();
    await expect(remoteHttp("https://example.test", {}, response => response.json(), controller.signal))
      .rejects.toThrow("remote_request_cancelled");
    expect(fetchMock).not.toHaveBeenCalled();
    // No deadline is left behind by a request that never started.
    expect(jest.getTimerCount()).toBe(0);
  });

  test("an abort that lands with the response wins over the response", async () => {
    jest.useFakeTimers();
    const controller = new AbortController();
    globalThis.fetch = jest.fn(async () => {
      controller.abort();
      return { json: async () => ({ ok: true }) } as unknown as Response;
    }) as unknown as typeof fetch;
    const consume = jest.fn(async () => "consumed");
    await expect(remoteHttp("https://example.test", {}, consume, controller.signal))
      .rejects.toThrow("remote_request_cancelled");
    // The body is never handed to the consumer after a cancellation.
    expect(consume).not.toHaveBeenCalled();
    expect(jest.getTimerCount()).toBe(0);
  });
});
