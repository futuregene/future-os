// Variant A · 选项行 + 说明 (recommended): one option chosen, its chip already in
// the composer. Shows the round trip — the card keeps the record, the chip is
// where the answer lives until it is sent.
(() => {
  const state = { picked: ["forest"] };
  const card = window.__choiceMount(state, "rows");
  // The card's own footer already says where the answer went; the composer chip
  // is what that sentence is talking about.
  window.__choiceComposerRow([
    window.__choiceChip("接下来", "重画森林图"),
  ]);
  return card.dataset.choiceCard;
})();
