// Exercise actual inline-handler templates through an HTML parser and JS parser.
import fs from 'node:fs';
import assert from 'node:assert/strict';
import { JSDOM } from 'jsdom';
const page = fs.readFileSync(new URL('../src/webui/page.rs', import.meta.url), 'utf8');
const esc = page.match(/^const esc = .*$/m)?.[0];
const jsArg = page.match(/^const jsArg = .*$/m)?.[0];
assert(esc && jsArg);
const encode = new Function(`${esc}\n${jsArg}\nreturn jsArg;`)();
const templates = [...page.matchAll(/onclick=(["'])((?:openGoal|inspectTodo)\([^\n]*?\))\1/g)].map(m => m[0]);
assert.equal(templates.length, 4, 'all dynamic goal/todo handler sites are covered');
const document = new JSDOM('').window.document;
for (const id of ['normal', "x');globalThis.__loopXss=true;//", 'x\" onmouseover=\"globalThis.__loopXss=true', "'&<>\n中\\"]) {
  for (const template of templates) {
    const attribute = new Function('g', 'i', 't', 'jsArg', `return \`${template}\`;`)({goal_id:id}, {goal_id:id}, {id}, encode);
    const host = document.createElement('div');
    host.innerHTML = `<button ${attribute}>click</button>`;
    const button = host.firstElementChild;
    assert.equal(button.attributes.length, 1, attribute);
    let actual;
    const capture = value => { actual = value; };
    new Function('openGoal', 'inspectTodo', button.getAttribute('onclick'))(capture, capture);
    assert.equal(actual, template.includes('openGoal') ? encodeURIComponent(id) : id);
    assert.equal(globalThis.__loopXss, undefined);
  }
}
console.log('WebUI attribute contexts: 16 actual-template HTML/JS round trips passed');
