/** One deadline covers fetch and body consumption; cancellation reaches fetch. */
export async function remoteHttp<T>(
  url: string,
  init: RequestInit,
  consume: (response: Response) => Promise<T>,
  signal?: AbortSignal,
): Promise<T> {
  const controller = new AbortController();
  let timedOut = false;
  const cancel = () => controller.abort();
  if (signal?.aborted) controller.abort();
  else signal?.addEventListener("abort", cancel, { once: true });
  const timer = setTimeout(() => {
    timedOut = true;
    cancel();
  }, 20_000);
  let rejectCancellation: (() => void) | undefined;
  try {
    if (controller.signal.aborted) throw new Error("remote_request_cancelled");
    const cancelled = new Promise<never>((_resolve, reject) => {
      rejectCancellation = () => reject(new Error("remote_request_cancelled"));
      controller.signal.addEventListener("abort", rejectCancellation, { once: true });
    });
    const request = (async () => {
      const response = await fetch(url, { ...init, signal: controller.signal });
      if (controller.signal.aborted) throw new Error("remote_request_cancelled");
      return consume(response);
    })();
    return await Promise.race([request, cancelled]);
  } catch (error) {
    if (timedOut && !signal?.aborted) throw new Error("network_request_timeout");
    throw error;
  } finally {
    clearTimeout(timer);
    signal?.removeEventListener("abort", cancel);
    if (rejectCancellation) controller.signal.removeEventListener("abort", rejectCancellation);
  }
}
