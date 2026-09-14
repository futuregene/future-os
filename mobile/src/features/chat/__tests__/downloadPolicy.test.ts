import * as Network from "expo-network";
import { DOWNLOAD_CONFIRM_BYTES, downloadWarning } from "../downloadPolicy";

jest.mock("expo-network", () => ({
  NetworkStateType: { WIFI: "wifi", ETHERNET: "ethernet", CELLULAR: "cellular", UNKNOWN: "unknown", NONE: "none", OTHER: "other" },
  getNetworkStateAsync: jest.fn(),
}));
const network = jest.mocked(Network.getNetworkStateAsync);
beforeEach(() => jest.resetAllMocks());

test.each([0, 5 * 1024, DOWNLOAD_CONFIRM_BYTES - 1])("small explicit download (%s bytes) does not query the network or prompt", async size => {
  await expect(downloadWarning(size)).resolves.toBeNull();
  expect(network).not.toHaveBeenCalled();
});

test.each([
  [Network.NetworkStateType.CELLULAR, "attachment.cellularWarning"],
  [Network.NetworkStateType.UNKNOWN, "attachment.unknownNetworkWarning"],
  [Network.NetworkStateType.NONE, "attachment.unknownNetworkWarning"],
  [Network.NetworkStateType.OTHER, "attachment.unknownNetworkWarning"],
  [Network.NetworkStateType.WIFI, null],
  [Network.NetworkStateType.ETHERNET, null],
] as const)("large transfers on %s receive the appropriate warning", async (type, expected) => {
  network.mockResolvedValue({ type });
  await expect(downloadWarning(DOWNLOAD_CONFIRM_BYTES)).resolves.toBe(expected);
  await expect(downloadWarning(10 * DOWNLOAD_CONFIRM_BYTES)).resolves.toBe(expected);
});

test("network detection failure still allows a large transfer after confirmation", async () => {
  network.mockRejectedValue(new Error("Network API unavailable"));
  await expect(downloadWarning(DOWNLOAD_CONFIRM_BYTES)).resolves.toBe("attachment.unknownNetworkWarning");
});

test.each([NaN, Infinity, -1])("invalid size %s does not bypass confirmation", async size => {
  network.mockResolvedValue({ type: Network.NetworkStateType.CELLULAR });
  await expect(downloadWarning(size)).resolves.toBe("attachment.cellularWarning");
});
