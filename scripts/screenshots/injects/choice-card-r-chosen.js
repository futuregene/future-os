// Remote conversation, state 2: a chosen answer in the remote composer's dock.
// The remote composer is a plain textarea with no pill support, so the answer
// goes in as a chip row above the field — the same place a local attachment sits.
(() => {
  const card = window.__choiceMount({ picked: ["forest"] }, "rows");
  window.__choiceComposerRow([
    window.__choiceChip("接下来", "重画森林图"),
  ]);
  return card.dataset.choiceCard;
})();
