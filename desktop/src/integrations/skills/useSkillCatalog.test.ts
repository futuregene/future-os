// @vitest-environment jsdom
import type { InstalledSkill } from "./skillsClient";
import { act } from "react";
import { beforeEach, expect, it, vi } from "vitest";
import { emitFutureEvent } from "../../lib/futureEvents";
import { flushAsync, renderHook } from "../../test/renderHook";
import { invokeCommand } from "../tauri/invoke";
import { installSkill, invalidateSkillCatalog } from "./skillsClient";
import { useSkillCatalog } from "./useSkillCatalog";

vi.mock("../tauri/invoke", () => ({ invokeCommand: vi.fn() }));
const invoke = vi.mocked(invokeCommand);
const skill: InstalledSkill = { id: "web", name: "web", description: "web", nameZh: null, descriptionZh: null, version: "1" };
let installed: InstalledSkill[];
beforeEach(() => {
  installed = [];
  invoke.mockReset();
  invalidateSkillCatalog();
  invoke.mockImplementation(async command => command === "list_installed_skills" ? installed : []);
});

it("shares reads across consumers and updates both after installation", async () => {
  const first = renderHook(() => useSkillCatalog());
  const second = renderHook(() => useSkillCatalog());
  try {
    await flushAsync();
    expect(invoke.mock.calls.filter(([command]) => command === "list_installed_skills")).toHaveLength(1);
    installed = [skill];
    await act(async () => installSkill("web", "1"));
    expect(first.current.installed).toEqual([skill]);
    expect(second.current.installed).toEqual([skill]);
    expect(invoke.mock.calls.filter(([command]) => command === "list_installed_skills")).toHaveLength(2);
  }
  finally {
    first.unmount();
    second.unmount();
  }
});

it("handles an external change with one invalidation for all consumers", async () => {
  const first = renderHook(() => useSkillCatalog());
  const second = renderHook(() => useSkillCatalog());
  try {
    await flushAsync();
    installed = [skill];
    await act(async () => emitFutureEvent("skills-changed", undefined));
    expect(first.current.installed).toEqual([skill]);
    expect(second.current.installed).toEqual([skill]);
    expect(invoke.mock.calls.filter(([command]) => command === "list_installed_skills")).toHaveLength(2);
  }
  finally {
    first.unmount();
    second.unmount();
  }
});
