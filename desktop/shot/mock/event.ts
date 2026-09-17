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
