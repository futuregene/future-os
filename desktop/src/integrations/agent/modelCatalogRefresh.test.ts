/**
 * Opening a model picker asks for a background catalogue refresh — at most once
 * per TTL window, and never at the cost of the picker itself.
 */
import { beforeEach, expect, it, vi } from "vitest";
import { MODEL_CATALOG_REFRESH_TTL_MS, requestModelCatalogRefresh, resetModelCatalogRefreshForTests } from "./modelCatalogRefresh";

const mocks = vi.hoisted(() => ({ requestFutureModelsRefresh: vi.fn() }));

vi.mock("./agentClient", () => ({ requestFutureModelsRefresh: mocks.requestFutureModelsRefresh }));

beforeEach(() => {
  resetModelCatalogRefreshForTests();
  mocks.requestFutureModelsRefresh.mockReset();
  mocks.requestFutureModelsRefresh.mockResolvedValue({ requested: true });
});

it("asks the agent on the first picker open", () => {
  expect(requestModelCatalogRefresh(1_000)).toBe(true);
  expect(mocks.requestFutureModelsRefresh).toHaveBeenCalledTimes(1);
});

it("stays quiet inside the TTL window and asks again after it", () => {
  expect(requestModelCatalogRefresh(1_000)).toBe(true);
  expect(requestModelCatalogRefresh(1_000 + MODEL_CATALOG_REFRESH_TTL_MS - 1)).toBe(false);
  expect(mocks.requestFutureModelsRefresh).toHaveBeenCalledTimes(1);

  expect(requestModelCatalogRefresh(1_000 + MODEL_CATALOG_REFRESH_TTL_MS)).toBe(true);
  expect(mocks.requestFutureModelsRefresh).toHaveBeenCalledTimes(2);
});

it("never surfaces a failed request at the picker", async () => {
  mocks.requestFutureModelsRefresh.mockRejectedValue(new Error("agent down"));
  // The call itself must not throw, task-not-catch style: it is fire-and-forget.
  expect(() => requestModelCatalogRefresh(1_000)).not.toThrow();
  // Let the rejected promise settle; an unhandled rejection would fail here.
  await Promise.resolve();
  await Promise.resolve();
});
