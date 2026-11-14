// Explored, not chosen: an explicit "write your own" row inside the card.
// Dropped — the hint line already points at the composer, and a dashed row that
// is not an input reads as a broken field.
(() => {
  return window.__choiceMount({ picked: [] }, "rows", { free: true }).dataset.choiceCard;
})();
