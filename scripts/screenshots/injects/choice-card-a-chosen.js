// v2 · one option chosen: the card keeps the record, the answer lives in the
// editor as a pill (the pill is what makes the send button live: `mentionDom.ts` treats any `[data-mention]` as content).
(() => {
  const card = window.__choiceMount({ picked: ["forest"] }, "rows");
  window.__choiceEditorPill("接下来", "重画森林图");
  return card.dataset.choiceCard;
})();
