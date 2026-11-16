// Explored, not chosen: numbered rows so the answer can be typed (`2`) instead
// of clicked. Dropped from the design — digits typed into the composer are free
// text, and giving them menu semantics makes "2 篇论文" ambiguous.
(() => {
  return window.__choiceMount({ picked: [] }, "rows", { numbers: true }).dataset.choiceCard;
})();
