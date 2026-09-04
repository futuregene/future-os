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
import { classifyNatsError } from "../natsErrors";
import { classifyError } from "../connectionState";
import { isTransientNatsRequestError } from "../client";

describe("NATS v3 error classification", () => {
  test.each([
    new TimeoutError(),
    new NoRespondersError("request.subject"),
    new ClosedConnectionError(),
    new ConnectionError("socket closed"),
    new RequestError("connection disconnected"),
    new RequestError("request failed", { cause: new NoRespondersError("request.subject") }),
  ])("retries %s without treating it as authentication failure", error => {
    expect(classifyNatsError(error)).toBe("transport");
    expect(isTransientNatsRequestError(error)).toBe(true);
    expect(classifyError(error)).toBe("transport");
  });

  test.each([
    new UserAuthenticationExpiredError("user authentication expired"),
    new AuthorizationError("account authentication expired"),
    new AuthorizationError("authorization violation"),
    new PermissionViolationError("permission denied", "publish", "request.subject"),
  ])("preserves authentication errors through request and diagnostic wrappers: %s", cause => {
    const error = new Error("nats_connect_failed", {
      cause: new RequestError("request failed", { cause }),
    });
    expect(classifyError(error)).toBe("auth");
    expect(isTransientNatsRequestError(error)).toBe(false);
  });

  test("distinguishes account expiry from other authorization errors", () => {
    expect(classifyNatsError(new AuthorizationError("Account Authentication Expired"))).toBe(
      "expired",
    );
    expect(classifyNatsError(new AuthorizationError("authentication timeout"))).toBe(
      "authorization",
    );
  });

  test("does not turn a protocol or application error into request recovery", () => {
    expect(
      isTransientNatsRequestError(
        new RequestError("failed", {
          cause: new ProtocolError("invalid protocol"),
        }),
      ),
    ).toBe(false);
    expect(isTransientNatsRequestError(new Error("application rejected request"))).toBe(false);
    expect(isTransientNatsRequestError(new Error("not_connected"))).toBe(true);
  });

  test("handles circular diagnostic causes", () => {
    const error = new Error("cycle");
    error.cause = error;
    expect(classifyNatsError(error)).toBeNull();
  });
});
