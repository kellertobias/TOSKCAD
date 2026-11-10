import test from 'node:test';import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';import {blueprint} from './website-blueprint.mjs';
const strip=svg=>svg.replace(/<!-- blueprint-presentation:start -->[\s\S]*?<!-- blueprint-presentation:end -->\n?/g,'');
test('all published drawing sheets retain source geometry and text through recolouring',()=>{
 const html=readFileSync(new URL('../website/index.html',import.meta.url),'utf8');
 const paths=[...new Set([...html.matchAll(/src="(assets\/[^"?]+\.svg)(?:\?[^"]*)?"/g)].map(m=>m[1]))];
 assert.equal(paths.length,10);
 for(const path of paths){const current=readFileSync(new URL('../website/'+path,import.meta.url),'utf8');const source=strip(current);assert.equal(strip(blueprint(source)),source);assert.equal(blueprint(current),current);assert.match(current,/fill="#143e70"/);assert.match(current,/text \{ fill: #ffffff;/);}
});
test('rejects invalid or nonpositive paper extents',()=>{
 for(const input of ['<html/>','<svg viewBox="0 0 0 10"/>','<svg viewBox="0 0 nope 10"/>'])assert.throws(()=>blueprint(input));
});
