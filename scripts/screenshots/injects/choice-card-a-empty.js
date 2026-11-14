// Final design · grouped rows: the card appended to the last reply, nothing
// chosen yet. The reference state for every other capture in this set.
(() => {
  return window.__choiceMount({ picked: [] }, "rows").dataset.choiceCard;
})();
