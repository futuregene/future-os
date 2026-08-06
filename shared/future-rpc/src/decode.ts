/**
 * Shared response/event decode for the TS clients.
 *
 * Migration semantics (mirrors the Rust `future_rpc::decode`): the agent
 * dual-writes the typed `payload` oneof alongside the JSON `data` string.
 * During the migration window these helpers decode `data` first — the TS
 * clients were written against its camelCase/number shape, and proto-loader
 * surfaces proto `int64` as String, so switching to the typed object first
 * would change number semantics. When the agent retires `data`, the typed
 * `payload` fallback below takes over automatically.
 */

/** Minimal shape of the proto-loader `RpcResponse` object we decode. */
export interface DecodableResponse {
  data?: unknown;
  payload?: unknown;
}

/** Minimal shape of the proto-loader `StreamEvent`/`ProjectedRunEvent`. */
export interface DecodableEvent {
  data?: unknown;
  payload?: unknown;
}

function safeParse(text: string): unknown {
  try {
    return JSON.parse(text);
  } catch {
    return text;
  }
}

/**
 * Extract the typed oneof member from a wrapper message, if present.
 * proto-loader (oneofs:true) exposes the chosen member name on the oneof's
 * virtual property (`kind`) and the member value under that name.
 */
function typedOneofMember(payload: unknown): unknown {
  if (!payload || typeof payload !== "object") {
    return undefined;
  }
  const record = payload as Record<string, unknown>;
  const kind = record.kind;
  if (typeof kind !== "string" || kind === "") {
    return undefined;
  }
  return record[kind];
}

/**
 * Decode a unary response payload to a plain value.
 *
 * Prefers the JSON `data` string (byte/shape-stable during the migration
 * window); falls back to the typed `payload` oneof when `data` is absent.
 */
export function responseData(response: DecodableResponse): unknown {
  const data = response.data;
  if (data != null) {
    return typeof data === "string" ? safeParse(data) : data;
  }
  const typed = typedOneofMember(response.payload);
  return typed !== undefined ? typed : data;
}

/**
 * Decode a stream event payload to the fields object the clients spread into
 * their event model. The redundant injected `type` key is dropped (the
 * envelope carries the type) — matching both the agent's typed
 * reconstruction and the clients' existing `{ type: _dropped, ...rest }`.
 */
export function streamEventData(event: DecodableEvent): Record<string, unknown> {
  let raw: unknown;
  if (event.data != null) {
    raw = typeof event.data === "string" ? (event.data === "" ? {} : safeParse(event.data)) : event.data;
  } else {
    raw = typedOneofMember(event.payload) ?? {};
  }
  if (raw && typeof raw === "object" && !Array.isArray(raw)) {
    const { type: _dropped, ...rest } = raw as Record<string, unknown>;
    return rest;
  }
  return {};
}
