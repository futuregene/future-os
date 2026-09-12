// Exercise the actual production worker bundle without a DOM. Node/jsdom
// imports select different package exports and miss browser-only dependencies.
import assert from "node:assert/strict";
import { readdir, readFile } from "node:fs/promises";
import { resolve, sep } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { runInNewContext } from "node:vm";

const assets = process.argv[2]
  ? pathToFileURL(`${resolve(process.argv[2])}${sep}`)
  : new URL("../dist/assets/", import.meta.url);
const files = (await readdir(assets)).filter(name => /^streamingMarkdown\.worker-.*\.js$/.test(name));
assert.equal(files.length, 1, "expected one production streaming Markdown worker");
const source = await readFile(new URL(files[0], assets), "utf8");
const replies = [];
const scope = { postMessage: message => replies.push(message) };
runInNewContext(source, scope, { filename: fileURLToPath(new URL(files[0], assets)), timeout: 10_000 });
const text = "# Heading &amp; entity\n\nA **paragraph**.\n\nTail";
scope.onmessage({ data: { id: 1, live: true, text } });
assert.equal(replies.length, 1);
assert.equal(replies[0].id, 1);
assert.equal(replies[0].text, text);
assert.equal(replies[0].blocks.length, 3);
assert.equal(replies[0].blocks.map(block => block.content).join(""), text);
assert.equal(replies[0].blocks[0].live, false);
assert.equal(replies[0].blocks.at(-1).live, true);
console.log("Production streaming Markdown worker: DOM-free startup and parsing passed.");
