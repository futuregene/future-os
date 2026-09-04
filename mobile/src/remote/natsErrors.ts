import {
  AuthorizationError,
  ClosedConnectionError,
  ConnectionError,
  NoRespondersError,
  PermissionViolationError,
  ProtocolError,
  RequestError,
  TimeoutError,
  UserAuthenticationExpiredError,
} from "@nats-io/nats-core";

type NatsFailure = "expired" | "authorization" | "protocol" | "transport";

/** Preserve typed NATS failures through request and diagnostic wrappers. */
export function classifyNatsError(value: unknown): NatsFailure | null {
  const seen = new Set<Error>();
  let error = value;
  let transport = false;
  while (error instanceof Error && !seen.has(error)) {
    seen.add(error);
    if (
      error instanceof UserAuthenticationExpiredError ||
      (error instanceof AuthorizationError &&
        error.message.toLowerCase().includes("account authentication expired"))
    ) {
      return "expired";
    }
    if (error instanceof AuthorizationError || error instanceof PermissionViolationError) {
      return "authorization";
    }
    if (error instanceof ProtocolError) return "protocol";
    transport ||=
      error instanceof TimeoutError ||
      error instanceof NoRespondersError ||
      error instanceof ClosedConnectionError ||
      error instanceof ConnectionError ||
      error instanceof RequestError;
    error = error.cause;
  }
  return transport ? "transport" : null;
}
