import { connectionPresentation } from "../connectionPresentation";
import type { ConnectionPhase } from "../types";

test.each<ConnectionPhase>(["booting", "claiming", "connecting", "reconnecting", "refreshing"])(
  "%s is temporarily unavailable",
  (phase) => {
    expect(connectionPresentation({ phase, desktopOnline: false })).toMatchObject({
      level: "connecting",
      customerState: "connecting",
      action: "wait",
      supportCode: null,
    });
  },
);
test("terminal states stay disconnected despite stale presence", () => {
  expect(
    connectionPresentation({ phase: "failed", desktopOnline: true, error: "timed out" }),
  ).toMatchObject({
    level: "disconnected",
    customerState: "networkUnavailable",
    action: "checkNetwork",
    supportCode: "NW002",
    titleKey: "connection.networkUnavailable",
  });
  expect(connectionPresentation({ phase: "revoked", desktopOnline: true })).toMatchObject({
    level: "disconnected",
    customerState: "pairingExpired",
    action: "pairAgain",
    supportCode: "PA001",
    titleKey: "connection.pairingExpired",
  });
  expect(connectionPresentation({ phase: "unpaired", desktopOnline: true })).toMatchObject({
    level: "disconnected",
    customerState: "disconnected",
    action: "reconnect",
    supportCode: null,
  });
  expect(connectionPresentation({ phase: "stopped", desktopOnline: true, desktopDisconnectReason: "user_disconnect" })).toMatchObject({
    level: "disconnected",
    customerState: "disconnected",
    action: "reconnect",
    hintKey: "connection.manuallyDisconnectedHint",
  });
});
test("requires usable desktop, distinguishes missing presence from explicit shutdown", () => {
  expect(connectionPresentation({ phase: "ready", desktopOnline: true })).toMatchObject({
    level: "connected",
    customerState: "connected",
  });
  expect(connectionPresentation({ phase: "ready", desktopOnline: false })).toMatchObject({
    level: "connecting",
    customerState: "waitingDesktop",
    action: "openDesktop",
    titleKey: "connection.waitingDesktop",
  });
  expect(
    connectionPresentation({ phase: "ready", desktopOnline: true, agentAvailable: false }),
  ).toMatchObject({
    level: "connecting",
    customerState: "devicePreparing",
    action: "restartDesktop",
    supportCode: "LC003",
    titleKey: "connection.waitingDesktop",
  });
  expect(
    connectionPresentation({ phase: "ready", desktopOnline: false, desktopDisconnected: true }),
  ).toMatchObject({ level: "disconnected", customerState: "disconnected" });
});

test.each([
  // A desktop that answers but cannot serve its history is a device problem.
  ["agent offline", "connection.deviceUnavailable"],
  ["agent is unavailable", "connection.deviceUnavailable"],
  // Credentials and capacity have their own titles; both are retryable states a
  // user can act on.
  ["401", "connection.pairingExpired"],
  ["503", "connection.serviceUnavailable"],
  // A missing/forbidden resource and an unclassifiable failure share the
  // generic "connection failed" title, which is all the phone can honestly say.
  ["404", "connection.connectionFailed"],
  ["sqlite row decode exploded", "connection.connectionFailed"],
])("a failed phone titled %s reads as %s", (error, titleKey) => {
  const shown = connectionPresentation({ phase: "failed", desktopOnline: true, error });
  expect(shown.titleKey).toBe(titleKey);
  expect(shown.hintKey).toBe("connection.failedHint");
  expect(shown.level).toBe("disconnected");
});

/**
 * The one state the transport cannot express.
 *
 * `connecting` and `reconnecting` are the client's words both for an attempt in
 * flight and for a pairing nothing has ever answered; a phone that has never
 * reached the desktop therefore sat on "connecting" with no end, which reads
 * exactly like "nearly there". The connection measures the silence and hands
 * the fact here, and here it becomes a terminal the user can act on.
 */
test("a desktop that never answered is a terminal, not another wait", () => {
  const shown = connectionPresentation({
    phase: "reconnecting",
    desktopOnline: false,
    desktopUnanswered: true,
  });
  expect(shown).toMatchObject({
    level: "disconnected",
    customerState: "desktopNotServing",
    action: "reconnect",
    supportCode: "RT007",
    titleKey: "connection.disconnected",
    hintKey: "connection.notServingHint",
  });
  // The same phone, one fact earlier: still a wait, so the two cannot collapse
  // into one reading.
  expect(connectionPresentation({ phase: "reconnecting", desktopOnline: false })).toMatchObject({
    level: "connecting",
    customerState: "connecting",
    supportCode: null,
  });
});

test.each<ConnectionPhase>(["booting", "claiming", "connecting", "reconnecting", "refreshing"])(
  "a %s phone with no answer at all is that same terminal",
  (phase) => {
    expect(connectionPresentation({ phase, desktopOnline: false, desktopUnanswered: true }))
      .toMatchObject({ customerState: "desktopNotServing", level: "disconnected" });
  },
);

test("an answer, a stated cause or a failure all outrank the unanswered flag", () => {
  // The desktop did answer: nothing about silence applies any more.
  expect(
    connectionPresentation({ phase: "ready", desktopOnline: true, desktopUnanswered: true }),
  ).toMatchObject({ level: "connected", customerState: "connected" });
  expect(
    connectionPresentation({ phase: "ready", desktopOnline: false, desktopUnanswered: true }),
  ).toMatchObject({ customerState: "waitingDesktop" });
  // The desktop said it was going away — a cause of its own.
  expect(
    connectionPresentation({ phase: "stopped", desktopOnline: false, desktopUnanswered: true }),
  ).toMatchObject({ customerState: "disconnected", supportCode: null });
  // The client gave up with an error the phone can classify.
  expect(
    connectionPresentation({ phase: "failed", desktopOnline: false, desktopUnanswered: true, error: "503" }),
  ).toMatchObject({ customerState: "serviceUnavailable", supportCode: "SV001" });
});
