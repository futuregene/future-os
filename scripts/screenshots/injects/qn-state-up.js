// State 2 for the demo sheet: press ↑ from inside the answer being read.
//
// The jump itself is a cut, not an animation, so the shot taken after this is
// the settled result: the question that starts the turn the reader was in,
// aligned just below the viewport's top edge (12px), with its row marked so the
// eye can find what moved.
const up = document.querySelector('[aria-label="上一个提问"]');
if (!up) throw new Error('question nav control is not on screen after scrolling');
if (up.getAttribute('aria-disabled') === 'true') throw new Error('↑ is not available');
up.click();
