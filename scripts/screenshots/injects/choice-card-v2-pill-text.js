// v2 · the answer pill, plus words typed after it in the same line.
//
// Requirement (c): the pill and free text coexist, and the serialized message is
// pill text first, then the words — document order, which is the order the
// reader sees.
(() => {
  const card = window.__choiceMount({ picked: ["forest"] }, "rows");
  window.__choiceEditorPill("接下来", "重画森林图");
  window.__choiceEditorText("顺便把图例挪到右下");
  return card.dataset.choiceCard;
})();
