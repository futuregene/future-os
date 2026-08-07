/**
 * RPC types for FutureAgent communication.
 *
 * Re-exported from the shared wire-contract package (@future-os/rpc) — the
 * single source of truth for the TS clients' RPC payload types. Keeping this
 * module as a re-export means existing `from "./types.js"` imports keep
 * working while the definitions live in one place.
 */
export type * from "@future-os/rpc";
