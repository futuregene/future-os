// State 3 for the demo sheet: press ↓ from inside the answer being read.
//
// Landed on the next question down, aligned the same way ↑ aligns — the pair is
// one ladder, so both directions put their target at the top of the screen.
const down = document.querySelector('[aria-label="下一个提问"]');
if (!down) throw new Error('question nav control is not on screen after scrolling');
if (down.getAttribute('aria-disabled') === 'true') throw new Error('↓ is not available');
down.click();
