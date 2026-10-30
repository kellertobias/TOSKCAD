import { readFileSync, writeFileSync, mkdirSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { resolve, join } from 'node:path';
import { execFileSync } from 'node:child_process';

const root = fileURLToPath(new URL('../', import.meta.url));
const source = join(root, 'assets/icons');
const catalog = JSON.parse(readFileSync(join(source, 'catalog.json'), 'utf8'));
const output = resolve(process.argv.find(a => a.startsWith('--out='))?.slice(6) ?? join(root, 'artifacts/icon-review'));
mkdirSync(output, { recursive: true });
const escape = value => value.replaceAll('&', '&amp;').replaceAll('<', '&lt;').replaceAll('"', '&quot;');
const render = (item, x, y, size, color) => {
  const svg = readFileSync(join(source, `${item.id}.svg`), 'utf8');
  if (!svg.includes('viewBox="0 0 64 64"') || !svg.includes('<title>') || !svg.includes('stroke-width="3"')) throw new Error(`Invalid source: ${item.id}`);
  const body = svg.replace(/^<svg[^>]*>/, '').replace(/<\/svg>\s*$/, '');
  return `<g transform="translate(${x} ${y}) scale(${size / 64})" fill="none" stroke="currentColor" stroke-width="3" stroke-linecap="round" stroke-linejoin="round" color="${item.success ? '#24aa60' : color}">${body}</g>`;
};
function sheet(title, items, filename) {
  const columns = 5, cellWidth = 210, cellHeight = 180;
  const width = columns * cellWidth + 32, height = Math.ceil(items.length / columns) * cellHeight + 100;
  let body = `<rect width="100%" height="100%" fill="#101821"/><text x="24" y="35" font-size="23" fill="#edf3f9">${escape(title)}</text><text x="24" y="60" font-size="12" fill="#adbfcd">64 px artwork · 24 / 16 px dark and light samples · proposals are labeled</text>`;
  items.forEach((item, i) => {
    const x = 16 + i % columns * cellWidth, y = 82 + Math.floor(i / columns) * cellHeight;
    body += `<rect x="${x + 3}" y="${y}" width="202" height="169" rx="8" fill="#1b2835"/>`;
    body += render(item, x + 73, y + 10, 64, '#edf3f9');
    body += `<text x="${x + 104}" y="${y + 96}" text-anchor="middle" fill="#edf3f9" font-size="12">${escape(item.label)}</text>`;
    body += `<text x="${x + 104}" y="${y + 112}" text-anchor="middle" fill="#9cafbf" font-size="10">${escape(item.status ?? 'Requested')}</text>`;
    body += render(item, x + 21, y + 128, 24, '#edf3f9') + render(item, x + 55, y + 132, 16, '#edf3f9');
    body += `<rect x="${x + 103}" y="${y + 122}" width="85" height="36" rx="4" fill="#f4f6f8"/>`;
    body += render(item, x + 115, y + 128, 24, '#15212c') + render(item, x + 151, y + 132, 16, '#15212c');
  });
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="${width}" height="${height}" viewBox="0 0 ${width} ${height}" font-family="Arial, sans-serif"><title>${escape(title)}</title>${body}</svg>\n`;
  writeFileSync(join(output, `${filename}.svg`), svg);
  if (process.argv.includes('--png')) execFileSync('rsvg-convert', [join(output, `${filename}.svg`), '-o', join(output, `${filename}.png`)]);
  return filename;
}
const sheets = Object.entries(Object.groupBy(catalog.icons, item => item.group)).map(([group, items]) => sheet(`ToskCAD / ${group}`, items, group.toLowerCase().replaceAll(' ', '-')));
sheet('ToskCAD / workbench icon review', catalog.icons, 'complete');
const html = `<!doctype html><html lang="en"><meta charset="utf-8"><title>ToskCAD icon review</title><style>body{background:#101821;color:#edf3f9;font:16px system-ui;margin:24px}img{display:block;width:min(100%,1082px);height:auto;margin:24px 0}a{color:#8dd6ff}</style><h1>ToskCAD icon review</h1><p>Artwork proposals only. New drawing modes, dropdowns and tools are not implemented by these assets.</p><p>Retain existing native motifs: ${escape(catalog.retained.join('; '))}.</p>${sheets.map(name => `<a href="${name}.svg">${name}</a><img src="${name}.svg" alt="${name} icon review">`).join('')}<p><a href="complete.svg">Complete sheet</a></p></html>`;
writeFileSync(join(output, 'index.html'), html);
console.log(`Validated ${catalog.icons.length} SVG sources; wrote ${sheets.length + 1} sheets and index.html to ${output}`);
