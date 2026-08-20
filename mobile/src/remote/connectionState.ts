/**
 * Connection lifecycle state machine (audit P2 — kills H4/M1/M7/M10).
 *
 * The old client handled every lifecycle event with a grab-bag of callbacks
 * and ad-hoc timer ownership: `close()` unconditionally broadcast
 * "disconnected" (arming a reconnect timer mid-refresh → H4's reconnect
 * storm), auth failures fell into the same retry path forever (M1), and a
 * failed JWT refresh leaked the previous NATS connection (M10). This module
 * replaces that with an explicit FSM: every lifecycle fact funnels through
 * `transition()`, each transition yields a small effect list, and the client
 * executes those effects — so timers have exactly one owner and leaving a
 * state disposes that state's resources.
 *
 * States:
 *   connecting   — a connect attempt is in flight (NATS connect + handshake).
 *   ready        — the live connection is usable.
 *   reconnecting — transport failed; NATS may still be retrying internally,
 *                  or a backoff timer (single owner) is armed for the next
 *                  attempt.
 *   refreshing   — a JWT refresh is rotating the credential mid-connection.
 *   revoked      — terminal: the desktop/server revoked this device. No
 *                  further network action ever fires from here.
 *   unpaired     — terminal: the user deregistered locally.
 *
 * `transport_disconnect` enters reconnecting WITHOUT arming the retry timer:
 * NATS's own status loop reconnects the same connection, and the `ready`
 * event arrives on success. Only an `open_failed` — a connect attempt that
 * actually died — arms the backoff timer. This is what makes H4's 1s
 * self-arming structurally impossible: close() never broadcasts a state that
 * arms a timer, and nothing re-arms while an attempt is in flight.
 */

export type ConnectionState =
  | "stopped"
  | "connecting"
  | "ready"
  | "reconnecting"
  | "refreshing"
  | "failed"
  | "revoked"
  | "unpaired";

export type LifecycleEvent =
  | { type: "open_started" }
  | { type: "open_failed"; error: Error }
  | { type: "ready" }
  | { type: "transport_disconnect" }
  | { type: "auth_failed" }
  | { type: "fatal"; error: Error }
  | { type: "revoked" }
  | { type: "unpair" };

export type ConnectionEffect =
  | { type: "dispose_connection"; reason: string }
  | { type: "schedule_reconnect" }
  | { type: "begin_token_refresh" }
  | { type: "enter_unpaired" };

export interface ConnectionAction {
  next: ConnectionState;
  effects: ConnectionEffect[];
}

export const MAX_BACKOFF_MS = 30_000;
const BASE_BACKOFF_MS = 1_000;

/** Exponential backoff with jitter — capped, and deterministic for tests. */
export function backoffDelayMs(attempt: number, random = Math.random): number {
  const exp = Math.min(attempt, 5); // 1s → 2s → 4s → 8s → 16s → 30s cap
  const base = BASE_BACKOFF_MS * 2 ** exp;
  const jitter = base * 0.2 * random();
  return Math.min(MAX_BACKOFF_MS, base + jitter);
}

/**
 * An HTTP failure from the remote control plane (claim / token refresh).
 * Carries the server's machine-readable `error` code separately from the human
 * `message`, so classification never has to sniff prose — mirroring the
 * desktop's `AppError::Remote { code, .. }`.
 */
export class RemoteApiError extends Error {
  readonly code?: string;
  readonly status: number;
  constructor(message: string, code: string | undefined, status: number) {
    super(message);
    this.name = "RemoteApiError";
    this.code = code;
    this.status = status;
  }
}

/**
 * Classify a connect/handshake/refresh failure into a lifecycle category.
 *   authTerminal — the device was revoked; stop every network action (M1).
 *   auth         — the token/pairing is broken but not revoked (refreshable).
 *   transport    — transient; retry with backoff.
 */
export function classifyError(error: unknown): "authTerminal" | "auth" | "fatal" | "transport" {
  // Control-plane HTTP errors carry the machine `code`; match that, never the
  // human message (mirrors the desktop's error_code classifier).
  if (error instanceof RemoteApiError) {
    if (error.code === "invalid_remote_credential" || error.code === "credentials_revoked") {
      return "authTerminal";
    }
    if (
      error.code === "pairing_signature_invalid" ||
      error.code === "pairing_confirmation_mismatch"
    ) {
      return "auth";
    }
    if (error.code === "remote_service_misconfigured") {
      return "fatal";
    }
    // Unknown server code — treat as retryable transport, matching the
    // desktop's generic "server" category.
    return "transport";
  }
  const message = error instanceof Error ? error.message : String(error);
  // A JWT with no readable exp is permanently malformed — refreshing can't
  // fix it, only re-pairing can (mirrors the desktop's hard reject).
  if (message.includes("invalid_jwt")) {
    return "authTerminal";
  }
  if (
    message.includes("pairing_signature_invalid") ||
    message.includes("pairing_confirmation_mismatch")
  ) {
    return "auth";
  }
  if (
    message.includes("PERMISSIONS_VIOLATION") ||
    message.includes("AUTHORIZATION_VIOLATION") ||
    message.includes("nats_authorization_rejected")
  ) {
    return "auth";
  }
  if (
    message.includes("remote_service_misconfigured") ||
    message.includes("generation_unhealthy")
  ) {
    return "fatal";
  }
  return "transport";
}

/**
 * Pure transition table. Returns the next state + the effects the client must
 * execute (timer arming, connection disposal, token refresh). Terminal states
 * never leave — a revoked device retries nothing (M1), and an unpaired device
 * only acts on a fresh pair.
 */
export function transition(current: ConnectionState, event: LifecycleEvent): ConnectionAction {
  switch (current) {
    case "connecting":
    case "reconnecting":
    case "refreshing":
      switch (event.type) {
        case "open_started":
          // A repeat attempt in flight — the oldest wins; the new one waits.
          return { next: current, effects: [] };
        case "ready":
          return { next: "ready", effects: [] };
        case "auth_failed":
          return { next: "refreshing", effects: [{ type: "begin_token_refresh" }] };
        case "revoked":
          return { next: "revoked", effects: [{ type: "dispose_connection", reason: "revoked" }] };
        case "fatal":
          return { next: "failed", effects: [{ type: "dispose_connection", reason: "fatal" }] };
        case "transport_disconnect":
          // Already reconnecting — absorb; only open_failed may arm the timer.
          return { next: "reconnecting", effects: [] };
        case "open_failed":
          return { next: "reconnecting", effects: [{ type: "schedule_reconnect" }] };
        case "unpair":
          return {
            next: "unpaired",
            effects: [{ type: "dispose_connection", reason: "unpair" }, { type: "enter_unpaired" }],
          };
      }
      break;
    case "ready":
      switch (event.type) {
        case "transport_disconnect":
          // NATS reconnects the same connection internally; no timer needed.
          return { next: "reconnecting", effects: [] };
        case "auth_failed":
          return { next: "refreshing", effects: [{ type: "begin_token_refresh" }] };
        case "revoked":
          return { next: "revoked", effects: [{ type: "dispose_connection", reason: "revoked" }] };
        case "fatal":
          return { next: "failed", effects: [{ type: "dispose_connection", reason: "fatal" }] };
        case "open_failed":
          return { next: "reconnecting", effects: [{ type: "schedule_reconnect" }] };
        case "unpair":
          return {
            next: "unpaired",
            effects: [{ type: "dispose_connection", reason: "unpair" }, { type: "enter_unpaired" }],
          };
        default:
          return { next: current, effects: [] };
      }
    case "revoked":
      // Terminal — nothing may transition out except a fresh pair.
      return { next: current, effects: [] };
    case "failed":
      // A manual reconnect creates a fresh RemoteClient; this failed instance
      // must not restart itself in the background.
      return { next: current, effects: [] };
    case "stopped":
      return { next: current, effects: [] };
    case "unpaired":
      // A fresh pair (a new RemoteClient with credentials) begins its first
      // open from here. Everything else is ignored.
      if (event.type === "open_started") {
        return { next: "connecting", effects: [] };
      }
      return { next: current, effects: [] };
  }
  return { next: current, effects: [] };
}
