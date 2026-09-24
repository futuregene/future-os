// Style A — two round buttons stacked on the right edge. This is the shape the
// product ships (mobile/src/features/chat/components/QuestionNavControl), so
// this file asserts the two properties the other panels are compared against
// rather than restyling anything: a vertical stack, and two 44px circular
// buttons — the platform's minimum touch target.
const up = document.querySelector('[aria-label="上一个提问"]');
const down = document.querySelector('[aria-label="下一个提问"]');
const stack = up && up.parentElement;

if (!up || !down || !stack) throw new Error('question nav control not found');
if (getComputedStyle(stack).flexDirection !== 'column')
  throw new Error(`expected a vertical stack, got ${getComputedStyle(stack).flexDirection}`);
if (up.parentElement !== down.parentElement)
  throw new Error('the two arrows are not in one container');

for (const [name, button] of [['↑', up], ['↓', down]]) {
  const style = getComputedStyle(button);
  const width = button.getBoundingClientRect().width;
  const height = button.getBoundingClientRect().height;
  if (Math.round(width) !== 44 || Math.round(height) !== 44)
    throw new Error(`${name} is ${Math.round(width)}×${Math.round(height)}, expected 44×44`);
  if (parseFloat(style.borderTopLeftRadius) < 20)
    throw new Error(`${name} is not circular (radius ${style.borderTopLeftRadius})`);
}
