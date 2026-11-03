import type { MachineStatus, MachineSummary } from "../../remote/desktopStatus";
import { needsDeviceStep, shareDevices } from "../shareDevices";

function machine(
  desktopId: string,
  patch: Partial<MachineSummary> = {},
): MachineSummary {
  return { desktopId, label: desktopId, active: false, status: "connected", ...patch };
}

test("keeps every paired machine, judged by the two facts a host needs at once", () => {
  const devices = shareDevices([
    machine("idle"),
    machine("held", { control: "standby" }),
    ...(["preparing", "connecting", "offline"] as MachineStatus[]).map(
      status => machine(status, { status }),
    ),
  ]);
  expect(devices.map(device => device.desktopId))
    .toEqual(["idle", "held", "preparing", "connecting", "offline"]);
  expect(devices.map(device => device.canTake))
    .toEqual([true, false, false, false, false]);
});

test("carries a machine's mark and name, and leaves the mark out when it has none", () => {
  const [named, plain] = shareDevices([
    machine("a", { label: "Office", icon: "laptop" }),
    machine("b"),
  ]);
  expect(named).toEqual({ desktopId: "a", label: "Office", icon: "laptop", canTake: true });
  expect(plain).not.toHaveProperty("icon");
});

test.each([
  {
    name: "one machine that can take it, the one being read",
    machines: [machine("a")],
    active: "a",
    expected: false,
  },
  {
    name: "two machines that can take it",
    machines: [machine("a"), machine("b")],
    active: "a",
    expected: true,
  },
  {
    name: "one machine that can take it, not the one being read",
    machines: [machine("a"), machine("b", { status: "offline" })],
    active: "b",
    expected: true,
  },
  {
    name: "no machine at all",
    machines: [] as MachineSummary[],
    active: "a",
    expected: false,
  },
])("$name: asks which machine = $expected", ({ machines, active, expected }) => {
  expect(needsDeviceStep(shareDevices(machines), active)).toBe(expected);
});
