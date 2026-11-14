// The answer pill, in the shape the product itself builds it.
//
// Not product code, and deliberately not a design prototype either: every class,
// attribute and payload below is copied from `MentionEditor.buildChoicePill`
// (`data-mention="choice"` + a `data-choice` JSON answer + the two 12px lucide
// glyphs). It exists because §11 step 3 lands the pill and the editor plumbing
// while the card click that fills it is step 4 — so the capture shows the real
// composer reacting to a real pill, and the assertion that matters (the send
// button) is the app's own reaction through `isEditorEmpty`, not a painted state.
(() => {
  const NS = "http://www.w3.org/2000/svg";
  const ANSWER = {
    mode: "single",
    options: [
      { id: "forest", label: "重画森林图" },
      { id: "flow", label: "补一张实验流程图" },
      { id: "table", label: "表里加一列样本量" },
    ],
    selectedIds: ["forest"],
    title: "接下来做哪一件？",
  };
  const ICON_PATHS = {
    change: [
      "M21.174 6.812a1 1 0 0 0-3.986-3.987L3.842 16.174a2 2 0 0 0-.5.83l-1.321 4.352a.5.5 0 0 0 .623.622l4.353-1.32a2 2 0 0 0 .83-.497z",
      "m15 5 4 4",
    ],
    remove: ["M18 6 6 18", "m6 6 12 12"],
  };

  const icon = (paths) => {
    const svg = document.createElementNS(NS, "svg");
    for (const [name, value] of Object.entries({
      fill: "none",
      height: "12",
      stroke: "currentColor",
      "stroke-linecap": "round",
      "stroke-linejoin": "round",
      "stroke-width": "2",
      viewBox: "0 0 24 24",
      width: "12",
    })) svg.setAttribute(name, value);
    for (const d of paths) {
      const path = document.createElementNS(NS, "path");
      path.setAttribute("d", d);
      svg.appendChild(path);
    }
    return svg;
  };

  window.__choicePill = () => document.querySelector('[data-mention="choice"]');

  window.__choiceInsertPill = () => {
    const editor = document.querySelector('[role="textbox"][contenteditable="true"]');
    if (!editor)
      throw new Error("the composer has no editable field");

    const pill = document.createElement("span");
    pill.className = "mx-0.5 inline-flex h-[21px] max-w-full items-center gap-0.5 rounded bg-accent-soft py-0 pr-1 pl-1.5 align-middle text-[0.92em] leading-[21px] text-accent";
    pill.setAttribute("data-mention", "choice");
    pill.setAttribute("data-choice", JSON.stringify(ANSWER));
    pill.setAttribute("contenteditable", "false");

    const label = document.createElement("span");
    label.className = "max-w-[18rem] truncate";
    label.textContent = ANSWER.options.find(option => option.id === ANSWER.selectedIds[0]).label;
    pill.appendChild(label);

    for (const action of ["change", "remove"]) {
      const button = document.createElement("button");
      button.type = "button";
      button.className = "flex size-4 shrink-0 cursor-pointer items-center justify-center rounded-sm opacity-70 hover:bg-surface hover:opacity-100";
      button.setAttribute("data-choice-action", action);
      button.setAttribute("aria-label", action === "change"
        ? `修改「${ANSWER.title}」的选择`
        : `移除「${ANSWER.title}」的选择`);
      button.appendChild(icon(action === "change" ? ICON_PATHS.change : ICON_PATHS.remove));
      pill.appendChild(button);
    }

    editor.appendChild(pill);
    editor.appendChild(document.createTextNode(" "));
    // The editor is non-controlled and reconciles from mutations; the input event
    // is the same one a browser-dispatched edit sends, so `syncEmpty` runs and the
    // composer's `inputEmpty` flips exactly as it does for a real insertion.
    editor.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "insertText" }));
    return "pill inserted";
  };

  return window.__choiceInsertPill();
})();
