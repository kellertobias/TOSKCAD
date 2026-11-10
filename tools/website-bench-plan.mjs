// Developer-only callouts over the native WB-103 sheet. The drawing geometry
// stays unchanged; measurements come from MCP list_parameters, not pixel scale.
import {readFileSync, writeFileSync} from 'node:fs';

const [drawing, parameters, output] = process.argv.slice(2);
if (!drawing || !parameters || !output) throw Error('Usage: website-bench-plan.mjs WB-103.svg PARAMETERS.json OUTPUT.svg');
let svg = readFileSync(drawing, 'utf8');
if (!svg.includes('WB-103')) throw Error('Expected the native worktop drawing WB-103');
const rows = JSON.parse(readFileSync(parameters, 'utf8')).parameters;
const mm = name => {
  const row = rows.find(row => row.name === name);
  if (row?.unit !== 'mm' || !Number.isFinite(row.value) || row.value <= 0) throw Error('Missing evaluated millimetre parameter: ' + name);
  return row.value;
};
const length = mm('top_length'), depth = mm('top_depth'), pitch = mm('hole_pitch');
const x = mm('hole_edge_x'), y = mm('hole_edge_y'), diameter = mm('hole_diameter');
if (x + 9 * pitch + diameter / 2 > length || y + 9 * pitch + diameter / 2 > depth) throw Error('The grid exceeds the worktop');
const lines = [
  `Worktop: ${length} × ${depth} mm`,
  `Grid: 10 × 10 / 100 × Ø${diameter} through`,
  `Pitch: ${pitch} mm in X and Y`,
  `First centre from left edge: ${x} mm`,
  `First centre from front edge: ${y} mm`,
];
const callouts = `<g id="evaluated-grid-callouts" font-family="Arial, Helvetica, sans-serif" fill="black"><text x="225" y="70" font-size="5" font-weight="bold">Bench-top drilling / mm</text>${lines.map((line, i) => `<text x="225" y="${83 + i * 8}" font-size="4">${line}</text>`).join('')}</g>`;
svg = svg.replace(/<g id="evaluated-grid-callouts"[\s\S]*?<\/g>/, '');
writeFileSync(output, svg.replace('</svg>', callouts + '\n</svg>'));
console.log('Annotated WB-103 with evaluated grid pitch, offsets and stock dimensions');
