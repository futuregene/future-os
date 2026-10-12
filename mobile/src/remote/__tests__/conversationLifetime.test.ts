import { captureConversationLifetime } from "../conversationLifetime";
import type { RemoteClient } from "../client";
import type { RemoteCredentials } from "../types";

function refs() {
  return {
    conversationEpochRef: { current: 0 },
    clientRef: { current: {} as RemoteClient | null },
    credentialsRef: { current: { pairId: "pair-A", expectedDesktopId: "desktop-A" } as RemoteCredentials | null },
  };
}

test("renewing credentials for the same pairing keeps an in-flight operation", () => {
  const state = refs();
  const isCurrent = captureConversationLifetime(state);
  state.credentialsRef.current = { ...state.credentialsRef.current!, userJwt: "renewed", refreshToken: "renewed" };
  expect(isCurrent()).toBe(true);
});

test("leaving and returning to a conversation cancels the original operation", () => {
  const state = refs();
  const isCurrent = captureConversationLifetime(state);
  state.conversationEpochRef.current += 2;
  expect(isCurrent()).toBe(false);
});

test("switching clients cancels work even when the pairing stays the same", () => {
  const state = refs();
  const isCurrent = captureConversationLifetime(state);
  state.clientRef.current = {} as RemoteClient;
  expect(isCurrent()).toBe(false);
});

test.each(["pairId", "expectedDesktopId"] as const)("changing %s cancels the operation", field => {
  const state = refs();
  const isCurrent = captureConversationLifetime(state);
  state.credentialsRef.current = { ...state.credentialsRef.current!, [field]: "another" };
  expect(isCurrent()).toBe(false);
});

test.each(["clientRef", "credentialsRef"] as const)("removing %s cancels the operation", field => {
  const state = refs();
  const isCurrent = captureConversationLifetime(state);
  state[field].current = null;
  expect(isCurrent()).toBe(false);
});
