// Phone, state 3: changing the choice — a bottom sheet, anchored to the chip.
(() => {
  window.__mchoice.chip("接下来", "重画森林图");
  return window.__mchoice.sheet(["forest"]).dataset.choiceSheet;
})();
