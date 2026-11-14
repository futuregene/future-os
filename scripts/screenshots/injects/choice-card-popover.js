// The chip row on its own, with the re-open popover anchored above it: what
// "the chosen card can be changed again" looks like.
(() => {
  const state = { picked: ["forest"] };
  window.__choiceComposerRow([window.__choiceChip("接下来", "重画森林图", { editing: true })]);
  window.__choicePopover(state);
  return "popover";
})();
