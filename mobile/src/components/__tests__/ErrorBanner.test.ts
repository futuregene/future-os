import { friendlyError } from "../errorMessage";

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
