// Variant styling for the demo sheet (see docs/guide/screenshots.md).
// A · the user bubble exactly as the app draws it today: the soft accent fill
// with a 1px accent hairline around it. Nothing is changed here — the inject
// only asserts the hairline is on screen, so a capture that lost it cannot
// silently be presented as "A".
const bubble = document.querySelector('div[class*="border-accent/25"]');
if (!bubble) throw new Error('no user bubble with an accent hairline is on screen');
const style = getComputedStyle(bubble);
if (parseFloat(style.borderTopWidth) === 0) throw new Error('the user bubble draws no border');
if (parseFloat(style.borderTopWidth) < 1) throw new Error('the user bubble hairline is thinner than a pixel');
