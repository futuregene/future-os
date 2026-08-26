const mockData = new Map<string, string>();

jest.mock("@react-native-async-storage/async-storage", () => ({
  getItem: jest.fn(async (key: string) => mockData.get(key) ?? null),
  setItem: jest.fn(async (key: string, value: string) => {
    mockData.set(key, value);
  }),
  removeItem: jest.fn(async (key: string) => {
    mockData.delete(key);
  }),
}));

import {
  clearPendingContinuation,
  loadPendingContinuation,
  savePendingContinuation,
  type PendingContinuation,
} from "../pendingContinuationStorage";

const pending: PendingContinuation = {
  version: 1,
  commandId: "continue-1",
  sessionId: "session-1",
  sourceRunId: "run-1",
  createdAt: 123,
};

describe("pending continuation storage", () => {
  beforeEach(() => mockData.clear());

  it("round trips the stable continuation identity", async () => {
    await savePendingContinuation(pending);
    await expect(loadPendingContinuation()).resolves.toEqual(pending);
  });

  it("ignores malformed records", async () => {
    mockData.set("futureos.remote.pending-continuation.v1", JSON.stringify({ version: 1 }));
    await expect(loadPendingContinuation()).resolves.toBeNull();
  });

  it("only clears the matching operation", async () => {
    await savePendingContinuation(pending);
    await clearPendingContinuation("stale-command");
    await expect(loadPendingContinuation()).resolves.toEqual(pending);
    await clearPendingContinuation(pending.commandId);
    await expect(loadPendingContinuation()).resolves.toBeNull();
  });
});
