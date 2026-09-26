import type { ConnectionPresentation } from "../connectionPresentation";
import { connectionPresentation } from "../connectionPresentation";
import { disconnectedCopy } from "../disconnectedCopy";
import { remoteErrorPresentation } from "../errorPresentation";

const presentation = (fields: Partial<ConnectionPresentation>): ConnectionPresentation => ({
  level: "disconnected",
  customerState: "disconnected",
  action: "reconnect",
  supportCode: null,
  titleKey: "connection.disconnected",
  hintKey: "connection.offlineHint",
  ...fields,
});

describe("the disconnected page names one cause and one next step", () => {
  test.each([
    ["user_disconnect", "manual"],
    ["system_sleep", "sleep"],
    ["app_exit", "exit"],
    ["system_power_off", "powerOff"],
    [undefined, "offline"],
    ["something_new_from_a_newer_desktop", "offline"],
  ] as [string | undefined, string][])(
    "a desktop that went away for %s reads as %s",
    (reason, expected) => {
      expect(disconnectedCopy(presentation({}), reason)).toBe(expected);
    },
  );

  test("the disconnect reason outranks any error or support code on the same frame", () => {
    expect(
      disconnectedCopy(presentation({ supportCode: "NW002" }), "user_disconnect", "recovery_timeout"),
    ).toBe("manual");
  });

  test("a recovery timeout is called out before the transport codes it also carries", () => {
    const shown = connectionPresentation({ phase: "failed", desktopOnline: true, error: "timed out" });
    expect(shown.supportCode).toBe("NW002");
    expect(disconnectedCopy(shown, undefined, "recovery_timeout")).toBe("recoveryTimeout");
  });

  test.each([
    [{ customerState: "serviceUnavailable", supportCode: "NW002" }, "timeout"],
    [{ customerState: "serviceUnavailable", supportCode: "SV002" }, "busy"],
    [{ customerState: "pairingExpired", action: "pairAgain" }, "pairing"],
    [{ customerState: "serviceUnavailable", action: "contactSupport" }, "support"],
    [{ customerState: "networkUnavailable" }, "network"],
    [{ customerState: "deviceUnavailable" }, "device"],
    [{ customerState: "serviceUnavailable" }, "service"],
    [{ customerState: "contentUnavailable" }, "content"],
    [{ customerState: "operationFailed" }, "failed"],
    [{ customerState: "pairingExpired" }, "failed"],
  ] as [Partial<ConnectionPresentation>, string][])(
    "a live connection failure of %o maps to %s",
    (fields, expected) => {
      expect(disconnectedCopy(presentation(fields))).toBe(expected);
    },
  );

  test("a support code outranks the action it comes with", () => {
    // The customer-facing cause is the code; the action only breaks ties.
    expect(
      disconnectedCopy(presentation({ customerState: "serviceUnavailable", action: "pairAgain", supportCode: "SV002" })),
    ).toBe("busy");
  });

  test("a null error is not mistaken for a recovery timeout", () => {
    expect(disconnectedCopy(presentation({}), "app_exit", null)).toBe("exit");
  });
});

describe("raw transport detail is classified into a customer category", () => {
  test.each([
    ["HTTP 401", "pairingExpired", "pairAgain", "connection.errorPairing", "PA002"],
    ["403", "pairingExpired", "pairAgain", "connection.errorPairing", "PA002"],
    ["404", "contentUnavailable", "retry", "connection.errorNotFound", "DT001"],
    ["429", "serviceUnavailable", "retry", "connection.errorRateLimit", "SV002"],
    ["500", "serviceUnavailable", "retry", "connection.errorServiceLater", "SV001"],
    ["HTTP 503", "serviceUnavailable", "retry", "connection.errorServiceLater", "SV001"],
  ])("%s is a status the phone understands", (message, customerState, action, messageKey, supportCode) => {
    expect(remoteErrorPresentation(message)).toEqual({ customerState, action, messageKey, supportCode });
  });

  test("surrounding whitespace does not change the reading", () => {
    expect(remoteErrorPresentation("   429  ").supportCode).toBe("SV002");
    expect(remoteErrorPresentation("\nHTTP 503\t").supportCode).toBe("SV001");
  });

  test.each([
    ["agent offline", "LC003", "deviceUnavailable"],
    ["Agent is unavailable", "LC003", "deviceUnavailable"],
    ["history is unavailable", "LC003", "deviceUnavailable"],
    ["request timed out", "NW002", "networkUnavailable"],
    ["time-out waiting for the desktop", "NW002", "networkUnavailable"],
    ["generation_unhealthy:protocol", "PT001", "serviceUnavailable"],
    ["generation_unhealthy:subscription", "RT001", "serviceUnavailable"],
    ["remote_service_misconfigured", "AU001", "serviceUnavailable"],
    ["permissions_violation", "AU001", "serviceUnavailable"],
    ["authorization_violation", "AU001", "serviceUnavailable"],
    ["network unreachable", "NW001", "networkUnavailable"],
    ["fetch failed", "NW001", "networkUnavailable"],
    ["ECONNREFUSED 10.0.0.4:443", "NW001", "networkUnavailable"],
    ["not_connected", "NW001", "networkUnavailable"],
    ["nats_connection_exhausted", "NW001", "networkUnavailable"],
    ["remote_state_subscription_ended", "NW001", "networkUnavailable"],
    ["connection refused", "NW001", "networkUnavailable"],
    ["connection reset by peer", "NW001", "networkUnavailable"],
    ["invalid_remote_credential", "PA001", "pairingExpired"],
    ["credentials_revoked", "PA001", "pairingExpired"],
    ["invalid_jwt", "PA001", "pairingExpired"],
    ["pairing_signature_mismatch", "PA001", "pairingExpired"],
    ["confirmation_mismatch", "PA001", "pairingExpired"],
    ["incomplete_desktop_credentials", "PA001", "pairingExpired"],
    ["desktop_credential_mismatch", "PA001", "pairingExpired"],
  ])("%s carries its support code %s", (message, supportCode, customerState) => {
    expect(remoteErrorPresentation(message)).toMatchObject({ supportCode, customerState });
  });

  test("a contact-support reading is what the escalation and authorization codes share", () => {
    expect(remoteErrorPresentation("generation_unhealthy:protocol").action).toBe("contactSupport");
    expect(remoteErrorPresentation("permissions_violation").action).toBe("contactSupport");
    // The subscription break is recoverable in place, so it retries instead.
    expect(remoteErrorPresentation("generation_unhealthy:subscription").action).toBe("retry");
  });

  test("a pairing expiry asks the user to pair again rather than to retry", () => {
    expect(remoteErrorPresentation("401")).toMatchObject({ action: "pairAgain", customerState: "pairingExpired" });
    expect(remoteErrorPresentation("invalid_remote_credential")).toMatchObject({
      action: "pairAgain", customerState: "pairingExpired",
    });
  });

  test.each([
    ["", "LC999"],
    ["  ", "LC999"],
    ["sqlite row decode exploded", "LC999"],
    ["HTTP 200", "LC999"],
    ["HTTP 1000", "LC999"],
    ["HTTP abc", "LC999"],
    // A bare status code is recognized; a longer number is not a status.
    ["5030", "LC999"],
  ])("an unrecognized failure (%j) is reported generically, never guessed at", (message, supportCode) => {
    expect(remoteErrorPresentation(message)).toEqual({
      customerState: "operationFailed",
      action: "retry",
      messageKey: "connection.errorGeneric",
      supportCode,
    });
  });

  test("the classification is a pure function of the message", () => {
    const message = "connection reset by peer";
    expect(remoteErrorPresentation(message)).toEqual(remoteErrorPresentation(message));
  });
});

describe("the whole disconnected page keeps status and copy in step", () => {
  test.each([
    ["failed", "timed out", "timeout"],
    ["failed", "invalid_remote_credential", "pairing"],
    ["failed", "generation_unhealthy:protocol", "support"],
    ["revoked", null, "pairing"],
    ["stopped", null, "offline"],
    ["unpaired", null, "offline"],
  ] as [Parameters<typeof connectionPresentation>[0]["phase"], string | null, string][])(
    "a %s phone with %s shows the %s copy",
    (phase, error, copy) => {
      const shown = connectionPresentation({ phase, desktopOnline: true, error });
      expect(disconnectedCopy(shown)).toBe(copy);
    },
  );

  test("a connected phone is never given disconnected copy", () => {
    const shown = connectionPresentation({ phase: "ready", desktopOnline: true });
    // "failed" is the fall-through for a state that is not one of the offline
    // causes; the page itself is only reachable while disconnected.
    expect(disconnectedCopy(shown)).toBe("failed");
  });
});
