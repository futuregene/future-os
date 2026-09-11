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
    titleKey: "connection.devicePreparing",
  });
  expect(
    connectionPresentation({ phase: "ready", desktopOnline: false, desktopDisconnected: true }),
  ).toMatchObject({ level: "disconnected", customerState: "disconnected" });
});
