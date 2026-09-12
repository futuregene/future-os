// Execute the actual embedded dashboard layout, not a second implementation.
import fs from 'node:fs';
import assert from 'node:assert/strict';
const page = fs.readFileSync(new URL('../src/webui/page.rs', import.meta.url), 'utf8');
const code = page.slice(page.indexOf('  const W = 218'), page.indexOf('  const stColor'));
const layout = new Function('layers', `${code}; return {pos,width,height,W,H};`);
for (const sizes of [[32], [8,1,1,1,1,5], Array(16).fill(1), [1]]) {
  const layers = sizes.map((size,l) => Array.from({length:size}, (_,r) => ({id:`${l}-${r}`})));
  const {pos,width,height,W,H} = layout(layers);
  const boxes = Object.values(pos);
  for (const p of boxes) { assert(p.x >= 0 && p.y >= 0); assert(p.x + W <= width); assert(p.y + H <= height); }
  for (let i=0;i<boxes.length;i++) for(let j=i+1;j<boxes.length;j++) {
    const a=boxes[i], b=boxes[j];
    assert(a.x+W<=b.x || b.x+W<=a.x || a.y+H<=b.y || b.y+H<=a.y, 'nodes overlap');
  }
}
console.log('graph layout: 4 actual-source fixtures passed');
