import { friendlyError, friendlyRunError } from "../errorMessage";

const t = (key: string): string => key;

describe("friendlyError", () => {
  test.each([
    ["HTTP 503", "connection.errorService"],
    ["nats_connection_exhausted", "connection.errorNetwork"],
    ["remote_state_subscription_ended", "connection.errorNetwork"],
    ["invalid_remote_credential", "connection.errorAuth"],
    ["pairing_confirmation_mismatch", "connection.errorAuth"],
  ])("maps %s to %s", (message, expected) => {
    expect(friendlyError(message, t)).toBe(expected);
  });

  test("does not expose an unknown internal error", () => {
    expect(friendlyError("sqlite row decode exploded", t)).toBe("connection.errorGeneric");
  });
});

describe("friendlyRunError", () => {
  test.each([
    ["Authentication failed (401). Check your API key.", "failure.auth"],
    ["API request failed (HTTP 429). Too many requests.", "failure.rateLimited"],
    ["API request failed (HTTP 503).", "failure.serverError"],
    ["[CTX_LIMIT] context too large", "failure.contextLimit"],
    ["Unable to connect to Future Agent", "failure.connect"],
  ])("maps %s to %s", (message, expected) => {
    expect(friendlyRunError(message, t)).toBe(expected);
  });

  test("falls back to the generic failure for unknown errors", () => {
    expect(friendlyRunError("model exploded", t)).toBe("failure.run");
    expect(friendlyRunError(undefined, t)).toBe("failure.unknown");
  });
});
