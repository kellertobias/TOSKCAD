import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {flatPlan} from './website-flat-plan.mjs';

const native = () => JSON.parse(readFileSync(new URL('../website/assets/front-panel-flat-pattern.json', import.meta.url), 'utf8'));

test('the actual panel drawing preserves every native opening and its measured blank', () => {
  const pattern = native();
  const svg = flatPlan(pattern);
  assert.equal(pattern.cut.loops.length, 16);
  assert.equal((svg.match(/<circle /g) ?? []).length, 14);
  assert.equal((svg.match(/stroke-dasharray="4 2"/g) ?? []).length, 2);
  // Independent dimensions and area: 80 mm front, two 40 mm flanges,
  // two 90-degree bend allowances, a 72x25 opening and fourteen holes.
  const allowance = Math.PI / 2 * (1.5 + 0.44 * 1.5);
  const height = 80 + 2 * 40 + 2 * allowance;
  const area = 260 * height - 72 * 25 - 6 * Math.PI * 6.25 ** 2 - 8 * Math.PI * 1.7 ** 2;
  assert.ok(Math.abs(pattern.size_mm[1] - height) < 1e-9);
  assert.ok(Math.abs(pattern.area_mm2 - area) < 1e-8);
  assert.match(svg, />260<\/text>/);
  assert.match(svg, />166\.786<\/text>/);
  assert.match(svg, />72 × 25<\/text>/);
  assert.match(svg, />6 × Ø12\.5<\/text>/);
  assert.match(svg, />8 × Ø3\.4<\/text>/);
});

test('rejects an open contour instead of silently closing or dropping geometry', () => {
  const pattern = native();
  pattern.cut.loops[1].edges[0].b[0] += 1;
  assert.throws(() => flatPlan(pattern), /Open or invalid contour/);
});

test('rejects unsupported arcs and mismatched native extents', () => {
  const arc = native();
  arc.cut.loops[2].edges[0].sweep_rad = Math.PI;
  assert.throws(() => flatPlan(arc), /Partial arcs/);
  const wrongSize = native();
  wrongSize.size_mm[0] += 5;
  assert.throws(() => flatPlan(wrongSize), /extents disagree/);
});
