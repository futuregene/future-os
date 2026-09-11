import { RecoveryCoordinator } from "../recoveryCoordinator";

function deferred() {
  let resolve!: () => void;
  const promise = new Promise<void>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}

test("batches notifications and performs one trailing pass after an in-flight reconnect", async () => {
  const coordinator = new RecoveryCoordinator();
  const owner = {};
  const gate = deferred();
  const recover = jest.fn().mockReturnValueOnce(gate.promise).mockResolvedValue(undefined);
  const first = coordinator.request(owner, () => true, recover);
  expect(coordinator.request(owner, () => true, recover)).toBe(first);
  await Promise.resolve();
  expect(recover).toHaveBeenCalledTimes(1);
  coordinator.request(owner, () => true, recover);
  coordinator.request(owner, () => true, recover);
  gate.resolve();
  await first;
  expect(recover).toHaveBeenCalledTimes(2);
});

test("retired client cannot run queued recovery or block a new client", async () => {
  const coordinator = new RecoveryCoordinator();
  const owner = {};
  const gate = deferred();
  let current = true;
  const recover = jest.fn(() => gate.promise);
  const first = coordinator.request(owner, () => current, recover);
  await Promise.resolve();
  coordinator.request(owner, () => current, recover);
  current = false;
  const next = jest.fn(async () => {});
  await coordinator.request({}, () => true, next);
  expect(next).toHaveBeenCalledTimes(1);
  gate.resolve();
  await first;
  expect(recover).toHaveBeenCalledTimes(1);
  await coordinator.request(owner, () => current, recover);
  expect(recover).toHaveBeenCalledTimes(1);
});

test("a failed pass releases its slot for later recovery", async () => {
  const coordinator = new RecoveryCoordinator();
  const owner = {};
  await expect(
    coordinator.request(
      owner,
      () => true,
      async () => {
        throw new Error("failed");
      },
    ),
  ).rejects.toThrow("failed");
  const recover = jest.fn(async () => {});
  await coordinator.request(owner, () => true, recover);
  expect(recover).toHaveBeenCalledTimes(1);
});
