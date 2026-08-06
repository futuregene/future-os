/**
 * @future-os/rpc — shared FutureAgent wire contract for the TS clients.
 *
 * Re-exports:
 *   - proto:   embedded schema + loader (`EMBEDDED_PROTO`, `loadAgentProto`)
 *   - decode:  `responseData` / `streamEventData` (data-first, typed fallback)
 *   - types:   merged RPC payload types
 */
export {
  EMBEDDED_PROTO,
  PROTO_LOADER_OPTIONS,
  resolveProtoPath,
  loadAgentProto,
} from "./proto.js";
export { responseData, streamEventData } from "./decode.js";
export type { DecodableResponse, DecodableEvent } from "./decode.js";
export * from "./types.js";
