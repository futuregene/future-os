// Runs the actual embedded browser script; a string-presence test misses closure bugs.
// Run: node cli/tests/console-hook.mjs
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';

const source = readFileSync(new URL('../src/browser/scripts.rs', import.meta.url), 'utf8');
const match = source.match(/pub const CONSOLE_HOOK_FUNCTION_SOURCE: &str = r#"([\s\S]*?)"#;/);
assert.ok(match, 'console hook source found');
const forwarded = [];
const target = { console: Object.fromEntries(['log', 'info', 'warn', 'error'].map(level => [level, (...args) => forwarded.push([level, ...args])])) };
runInNewContext(`(${match[1]})()`, target);
for (const level of ['log', 'info', 'warn', 'error']) target.console[level](level);
assert.deepEqual(Array.from(target.__futureConsoleLogs, entry => entry.level), ['log', 'info', 'warn', 'error']);
assert.deepEqual(forwarded.map(entry => entry[0]), ['log', 'info', 'warn', 'error']);
console.log('console hook preserves all four levels and original methods');
