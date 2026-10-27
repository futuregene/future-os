// Variant styling for the demo sheet (see docs/guide/screenshots.md).
// B · the same bubble with the hairline taken off: the soft accent fill alone.
// A stylesheet rather than a per-node style, because a React re-render restores
// a mutated `style` attribute but never removes a rule from the page. The box is
// untouched — only the border's colour goes transparent — so A and B differ in
// one property and nothing shifts between the two panels.
const id = 'shot-user-bubble-border-off';
if (!document.getElementById(id)) {
  const style = document.createElement('style');
  style.id = id;
  style.textContent = 'div[class*="border-accent/25"]{border-color:transparent !important}';
  document.head.appendChild(style);
}
const bubble = document.querySelector('div[class*="border-accent/25"]');
if (!bubble) throw new Error('no user bubble is on screen to restyle');
const colour = getComputedStyle(bubble).borderTopColor;
if (colour !== 'rgba(0, 0, 0, 0)' && colour !== 'transparent')
  throw new Error('the hairline did not go transparent: ' + colour);
