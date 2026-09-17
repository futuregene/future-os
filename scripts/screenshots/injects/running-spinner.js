// Variant styling for the demo sheet (see docs/guide/screenshots.md).
// Replaces every `svg` inside the run-status chip so the same screen can be
// rendered with a few icon options. This is a mock-up overlay, not a product
// change: the chosen style still has to be implemented in the app.
const svg = (body) => '<svg viewBox="0 0 24 24" width="14" height="14" '
  + 'fill="currentColor" stroke="currentColor" style="display:block">' + body + '</svg>';
const chip = document.querySelector('[aria-label="工具调用 5 次 · 思考 1 次"]');
if (chip) {
  chip.innerHTML = svg("<path d=\"M12 5a7 7 0 1 0 7 7\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"2.5\" stroke-linecap=\"round\"/>")
    + '<span style="margin-left:4px">×5 · ×1</span>';
}
