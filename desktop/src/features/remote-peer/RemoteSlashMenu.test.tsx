// @vitest-environment jsdom
import type { RemoteSkillChoice, RemoteSlashMenuGroups } from "./remoteCommands";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { RemoteSlashMenu } from "./RemoteSlashMenu";

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

/**
 * The `/` menu a remote composer opens.
 *
 * Above the box rather than below it: the box sits at the foot of the window, so
 * a menu opening downward would open off-screen.
 */
async function mount(groups: Partial<RemoteSlashMenuGroups> = {}, activeIndex = 0) {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const onChoose = vi.fn();
  const command = { description: "Compact this conversation's context.", id: "compact", label: "Compact context", run: vi.fn() };
  const skill: RemoteSkillChoice = {
    name: "future-web",
    description: "Search the public web and verify facts.",
    descriptionZh: "搜索公开网页并核实信息",
  };
  await act(async () => {
    root.render(
      <RemoteSlashMenu
        activeIndex={activeIndex}
        groups={{ commands: [command], skills: [skill], ...groups }}
        onChoose={onChoose}
      />,
    );
  });
  return {
    command,
    container,
    onChoose,
    options: () => [...container.querySelectorAll<HTMLButtonElement>("[role='option']")],
    skill,
    unmount: async () => {
      await act(async () => root.unmount());
      container.remove();
    },
  };
}

it("lists the host's commands and skills, and reports the chosen one", async () => {
  const view = await mount();
  expect(view.container.querySelector("[role='listbox']")).not.toBeNull();
  // A skill is offered by the name the host resolves, with what it does beside it.
  expect(view.container.textContent).toContain("Compact context");
  expect(view.container.textContent).toContain("/future-web");
  expect(view.container.textContent).toContain("Search the public web and verify facts.");

  await act(async () => view.options()[0]!.click());
  expect(view.onChoose).toHaveBeenCalledWith({ kind: "command", command: view.command });
  await act(async () => view.options()[1]!.click());
  expect(view.onChoose).toHaveBeenCalledWith({ kind: "skill", skill: view.skill });
  await view.unmount();
});

/**
 * The two groups are told apart by a heading, and the panel's own title names
 * whatever leads it: with a command first that is "命令", and with skills only
 * (a draft, where nothing can be compacted) it is "技能".
 */
it("heads each group, and titles the panel after whatever leads it", async () => {
  const both = await mount();
  const text = both.container.textContent ?? "";
  expect(text).toContain("Commands");
  // The skills' heading is *between* the groups: after the command row it
  // separates, and before the skill it heads.
  expect(text.indexOf("Skills")).toBeGreaterThan(text.indexOf("Compact context"));
  expect(text.indexOf("Skills")).toBeLessThan(text.indexOf("/future-web"));
  await both.unmount();

  const skillsOnly = await mount({ commands: [] });
  expect(skillsOnly.container.textContent).toContain("Skills");
  expect(skillsOnly.container.textContent).not.toContain("Commands");
  await skillsOnly.unmount();
});

/** The keyboard's position is the one the menu draws as chosen. */
it("marks the active row across both groups", async () => {
  const first = await mount({}, 0);
  expect(first.options().map(row => row.getAttribute("aria-selected"))).toEqual(["true", "false"]);
  await first.unmount();

  // The index is the flat one: 1 is the first skill, not the second command.
  const second = await mount({}, 1);
  expect(second.options().map(row => row.getAttribute("aria-selected"))).toEqual(["false", "true"]);
  await second.unmount();
});

/**
 * Choosing a row must not move the caret out of the field: the chosen skill is
 * inserted where the word was typed, and the message continues from there. The
 * local menu's rows prevent the mousedown for the same reason.
 */
it("keeps the field's focus when a row is pressed", async () => {
  const view = await mount();
  const mousedown = new MouseEvent("mousedown", { bubbles: true, cancelable: true });
  await act(async () => view.options()[0]!.dispatchEvent(mousedown));

  expect(mousedown.defaultPrevented).toBe(true);
  await view.unmount();
});

/** Nothing to offer means nothing drawn — not an empty panel. */
it("draws nothing when no command matches", async () => {
  const container = document.createElement("div");
  const root = createRoot(container);
  await act(async () => {
    root.render(<RemoteSlashMenu activeIndex={0} groups={{ commands: [], skills: [] }} onChoose={vi.fn()} />);
  });
  expect(container.querySelector("[role='listbox']")).toBeNull();
  await act(async () => root.unmount());
});
