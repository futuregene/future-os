// State 3 for the demo sheet: press ↓ from inside the answer being read.
//
// Landed on the next item down, aligned the way ↑ aligns its own target: one
// landing geometry for both directions, so a ↓ step stops at the top of the
// screen just like a prompt does. ↓ walks the transcript, so its target may be
// the turn's answer rather than a question.
const down = document.querySelector('[aria-label="下一段"]');
if (!down) throw new Error('question nav control is not on screen after scrolling');
if (down.getAttribute('aria-disabled') === 'true') throw new Error('↓ is not available');
down.click();
