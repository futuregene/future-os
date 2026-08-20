import { friendlyError, friendlyRunError } from "../errorMessage";

const t = (key: string, opts?: Record<string, unknown>): string =>
  opts?.code ? `${key}:${String(opts.code)}` : key;

describe("friendlyError", () => {
  test.each([
    ["HTTP 503", "connection.errorServiceLater:SV001"],
    ["nats_connection_exhausted", "connection.errorNetwork:NW001"],
    ["remote_state_subscription_ended", "connection.errorNetwork:NW001"],
    ["invalid_remote_credential", "connection.errorPairing:PA001"],
    ["pairing_confirmation_mismatch", "connection.errorPairing:PA001"],
    ["generation_unhealthy:protocol", "connection.errorServiceSupport:PT001"],
    ["generation_unhealthy:subscription", "connection.errorServiceLater:RT001"],
  ])("maps %s to %s", (message, expected) => {
    expect(friendlyError(message, t)).toBe(expected);
  });

  test("does not expose an unknown internal error", () => {
    expect(friendlyError("sqlite row decode exploded", t)).toBe("connection.errorGeneric:LC999");
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
