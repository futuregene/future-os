// Variant C · pills: the whole question as one wrapped row of tags. Cheapest to
// scan and the only variant that survives a narrow width, but a long label wraps
// mid-phrase.
(() => {
  return window.__choiceMount({ picked: [] }, "pills").dataset.choiceCard;
})();
