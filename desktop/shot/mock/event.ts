/** Mock of `@tauri-apps/api/event` — in-page event bus plus a no-op emitter. */
type Handler = (event: { event: string; id: number; payload: any }) => void;

const listeners = new Map<string, Set<Handler>>();
let nextId = 1;

export async function listen<T>(
  event: string,
  handler: (event: { event: string; id: number; payload: T }) => void,
  _options?: unknown,
): Promise<() => void> {
  const set = listeners.get(event) ?? new Set<Handler>();
  listeners.set(event, set);
  const entry: Handler = handler as Handler;
  set.add(entry);
  return () => {
    set.delete(entry);
  };
}

export async function once<T>(
  event: string,
  handler: (event: { event: string; id: number; payload: T }) => void,
): Promise<() => void> {
  return listen(event, handler);
}

export async function emit(event: string, payload?: unknown): Promise<void> {
  for (const handler of listeners.get(event) ?? []) {
    handler({ event, id: nextId++, payload });
  }
}

export async function emitTo(): Promise<void> {}

export class TauriEvent {
  static readonly WINDOW_RESIZED = "tauri://resize";
}

/**
 * The page's own handle on the bus, for the UI harness.
 *
 * In a real launch these events come from the Rust side — the host publishes
 * `tasks-updated` after a remote write, the agent publishes run progress — and
 * nothing inside the page can produce one. The harness has no Rust side, so a
 * case that is about "the window learns about a change made elsewhere" delivers
 * the event here, where the app really listens for it.
 */
if (typeof window !== "undefined")
  (window as unknown as { __shotEmit?: typeof emit }).__shotEmit = emit;
