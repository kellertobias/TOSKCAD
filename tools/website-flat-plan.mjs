// Developer-only presentation of native unfold_sheet output. No CAD geometry
// is inferred: this renderer accepts closed straight contours and full circles.
import {readFileSync, writeFileSync} from 'node:fs';
import {fileURLToPath} from 'node:url';
import {resolve} from 'node:path';

const xml = value => String(value).replaceAll('&', '&amp;').replaceAll('<', '&lt;').replaceAll('>', '&gt;').replaceAll('"', '&quot;');
const near = (a, b) => Math.abs(a - b) < 1e-6;
const point = p => Array.isArray(p) && p.length === 2 && p.every(Number.isFinite);
const same = (a, b) => near(a[0], b[0]) && near(a[1], b[1]);
const n = value => Number(value.toFixed(3));

export function flatPlan(pattern, logo = '') {
  if (pattern.units !== 'mm' || !pattern.cut?.loops?.length || !point(pattern.size_mm) ||
      !Number.isFinite(pattern.thickness_mm) || !Number.isFinite(pattern.k_factor)) throw Error('Expected native unfolded geometry in millimetres');
  const loops = pattern.cut.loops;
  if (loops.filter(loop => loop.outer).length !== 1) throw Error('Expected exactly one outer contour');
  const outer = loops.find(loop => loop.outer);
  if (!outer.edges.length || outer.edges.some(edge => edge.kind !== 'line')) throw Error('Outer contour must contain straight edges');
  const points = outer.edges.flatMap(edge => [edge.a, edge.b]);
  if (!points.every(point)) throw Error('Invalid contour coordinates');
  const min = [0, 1].map(axis => Math.min(...points.map(p => p[axis])));
  const max = [0, 1].map(axis => Math.max(...points.map(p => p[axis])));
  const size = max.map((value, axis) => value - min[axis]);
  if (!size.every((value, axis) => near(value, pattern.size_mm[axis]) && value > 0)) throw Error('Contour extents disagree with native dimensions');
  const scale = Math.min(1, 280 / size[0], 175 / size[1]);
  const x = value => 55 + (value - min[0]) * scale;
  const y = value => 62 + (max[1] - value) * scale;
  const shapes = [];
  const dimensions = [];
  const circles = [];
  for (const loop of loops) {
    if (!loop.edges?.length) throw Error('Empty contour');
    for (let i = 0; i < loop.edges.length; i++) {
      const edge = loop.edges[i];
      if (!point(edge.a) || !point(edge.b) || !same(edge.b, loop.edges[(i + 1) % loop.edges.length].a)) throw Error('Open or invalid contour');
    }
    if (loop.edges.length === 1 && loop.edges[0].kind === 'arc') {
      const edge = loop.edges[0];
      if (!point(edge.center) || !near(Math.abs(edge.sweep_rad), Math.PI * 2) || !same(edge.a, edge.b)) throw Error('Partial arcs are not supported');
      const radius = Math.hypot(edge.a[0] - edge.center[0], edge.a[1] - edge.center[1]);
      if (!(radius > 0)) throw Error('Invalid circle radius');
      circles.push(radius * 2);
      shapes.push(`<circle cx="${x(edge.center[0])}" cy="${y(edge.center[1])}" r="${radius * scale}"/>`);
    } else {
      if (loop.edges.some(edge => edge.kind !== 'line')) throw Error('Unsupported contour edge');
      shapes.push(`<path d="${loop.edges.map((edge, i) => `${i ? 'L' : 'M'}${x(edge.a[0])},${y(edge.a[1])}`).join(' ')} Z"/>`);
      if (!loop.outer && loop.edges.length === 4 && loop.edges.every(edge => near(edge.a[0], edge.b[0]) || near(edge.a[1], edge.b[1]))) {
        const xs = loop.edges.map(edge => edge.a[0]), ys = loop.edges.map(edge => edge.a[1]);
        const lo = [Math.min(...xs), Math.min(...ys)], hi = [Math.max(...xs), Math.max(...ys)];
        dimensions.push(`<text x="${x((lo[0] + hi[0]) / 2)}" y="${y((lo[1] + hi[1]) / 2) + 1.3}" text-anchor="middle">${n(hi[0] - lo[0])} × ${n(hi[1] - lo[1])}</text>`);
      }
    }
  }
  const bends = pattern.bends.map((bend, i) => {
    if (!Array.isArray(bend.line_mm) || bend.line_mm.length !== 2 || !bend.line_mm.every(point) ||
        !Number.isFinite(bend.angle_deg) || !Number.isFinite(bend.inside_radius_mm) || !['up', 'down'].includes(bend.direction)) throw Error('Invalid native bend');
    const [a, b] = bend.line_mm;
    return `<path d="M${x(a[0])},${y(a[1])} L${x(b[0])},${y(b[1])}" stroke-dasharray="4 2"/>
<text x="340" y="${(y(a[1]) + y(b[1])) / 2 - 2}" stroke="none" fill="#174f75">B${i + 1}: ${n(bend.angle_deg)}° ${xml(bend.direction)}</text>
<text x="340" y="${(y(a[1]) + y(b[1])) / 2 + 4}" stroke="none" fill="#174f75">Inside R${n(bend.inside_radius_mm)}</text>`;
  });
  const left = x(min[0]), right = x(max[0]), top = y(max[1]), bottom = y(min[1]);
  const holeGroups = new Map();
  for (const diameter of circles) holeGroups.set(n(diameter), (holeGroups.get(n(diameter)) ?? 0) + 1);
  const holeNotes = [...holeGroups].map(([diameter, count], i) => `<text x="340" y="${153 + i * 7}">${count} × Ø${diameter}</text>`).join('\n');
  return `<svg xmlns="http://www.w3.org/2000/svg" width="420mm" height="297mm" viewBox="0 0 420 297" role="img" aria-labelledby="title desc">
<title id="title">Front panel: unfolded sheet drawing</title>
<desc id="desc">Native developed contours, ${loops.length - 1} internal cutouts and ${pattern.bends.length} bend lines. Dimensions in millimetres.</desc>
<rect width="420" height="297" fill="white"/>
<g font-family="Arial, Helvetica, sans-serif" fill="#111" font-size="4">
<text x="20" y="23" font-size="9" font-weight="bold">Front panel / unfolded sheet</text>
<text x="20" y="33">${xml(pattern.name)} · all dimensions in mm · scale ${scale === 1 ? '1:1' : `1:${n(1 / scale)}`}</text>
<g fill="none" stroke="#111" stroke-width="0.4">${shapes.join('\n')}</g>
<g fill="none" stroke="#174f75" stroke-width="0.4">${bends.join('\n')}</g>
${dimensions.join('\n')}
<text x="340" y="145" font-weight="bold">Circular openings</text>
${holeNotes}
<g fill="none" stroke="#333" stroke-width="0.25">
<path d="M${left},${top - 2} V42 M${right},${top - 2} V42 M${left},46 H${right}"/>
<path d="M${left},46 l3,-1.2 v2.4 Z M${right},46 l-3,-1.2 v2.4 Z" fill="#333"/>
<path d="M${left - 2},${top} H33 M${left - 2},${bottom} H33 M37,${top} V${bottom}"/>
<path d="M37,${top} l-1.2,3 h2.4 Z M37,${bottom} l-1.2,-3 h2.4 Z" fill="#333"/>
</g>
<text x="${(left + right) / 2}" y="43" text-anchor="middle">${n(size[0])}</text>
<text transform="translate(33 ${(top + bottom) / 2}) rotate(-90)" text-anchor="middle">${n(size[1])}</text>
<text x="55" y="247">Thickness ${n(pattern.thickness_mm)} mm · K-factor ${n(pattern.k_factor)} · ${pattern.bends.length} bends</text>
<text x="55" y="255">Solid lines: cut edges. Dashed lines: bend centres.</text>
<text x="55" y="265">Bend IDs follow model feature order, not a press-brake sequence.</text>
<text x="55" y="273">Verify tooling, bend deductions and hardware fit before fabrication.</text>
${logo ? `<image x="340" y="254" width="58" height="19.5" href="data:image/png;base64,${logo}"/>` : ''}
</g></svg>\n`;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const [input, output] = process.argv.slice(2);
  if (!input || !output) throw Error('Usage: website-flat-plan.mjs UNFOLD.json OUTPUT.svg');
  const pattern = JSON.parse(readFileSync(input, 'utf8'));
  const logo = readFileSync(new URL('../website/assets/logo-monochrome.png', import.meta.url)).toString('base64');
  writeFileSync(output, flatPlan(pattern, logo));
  console.log(`Wrote ${output}: ${pattern.cut.loops.length} native contours, ${pattern.bends.length} bends`);
}
