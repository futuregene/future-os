// v2 · the answer pill inside the editor, change popover open above it.
//
// The pencil lives on the pill itself (in the editor), so the popover anchors to
// the pill's own rect: on the desktop there is room to open in place, which is
// exactly the thing the phone cannot do (see the phone's bottom sheet).
(() => {
  window.__choiceMount({ picked: ["forest"] }, "rows");
  window.__choiceEditorPill("接下来", "重画森林图", { editing: true });
  window.__choicePopover({ picked: ["forest"] }, { anchor: "[data-mention='choice']" });
  return "popover";
})();
