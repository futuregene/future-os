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
assert.equal(replies[0].blocks[0].parsed, true);
assert.equal(replies[0].blocks[0].document.nodes[0].type, "heading");
assert.equal(replies[0].blocks[1].document.nodes[0].children[1].type, "strong");
const table = "| A | B |\n| --- | --- |\n" + "| stable | row |\n".repeat(500) + "| mutable | **bo";
scope.onmessage({ data: { id: 2, live: true, text: table } });
const appended = table + "ld** |\n| next | row |";
scope.onmessage({ data: { id: 3, live: true, text: appended } });
assert.equal(replies.at(-1).blocks[0].document.nodes[0].rows.length, 502);
assert.equal(replies.at(-1).blocks[0].document.nodes[0].rows[500][1][0].type, "strong");
scope.onmessage({ data: { id: 4, live: false, text: appended } });
assert.equal(replies.at(-1).blocks[0].live, false);
assert.equal(replies.at(-1).blocks[0].document.raw, appended);
console.log("Production streaming Markdown worker: DOM-free startup, incremental table parsing and finalization passed.");
