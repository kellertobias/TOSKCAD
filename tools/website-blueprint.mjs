// Presentation-only recolouring of owned vector drawing assets. Geometry,
// dimensions, line weights and embedded logo pixels remain in their source form.
import {readFileSync,writeFileSync} from 'node:fs';
import {resolve} from 'node:path';
import {fileURLToPath} from 'node:url';

export function blueprint(svg) {
  if (!/<svg\b/.test(svg) || !/viewBox="[^"]+"/.test(svg)) throw Error('Expected a vector drawing with a viewBox');
  const viewBox=svg.match(/viewBox="([^"]+)"/)[1].split(/\s+/).map(Number);
  if(viewBox.length!==4||viewBox.some(v=>!Number.isFinite(v))||viewBox[0]!==0||viewBox[1]!==0||viewBox[2]<=0||viewBox[3]<=0)throw Error('Unsupported drawing extent');
  // Reapplying the tool replaces its presentation block, never adds geometry.
  svg=svg.replace(/<!-- blueprint-presentation:start -->[\s\S]*?<!-- blueprint-presentation:end -->\n?/g,'');
  const presentation=`<!-- blueprint-presentation:start -->
<defs><filter id="blueprint-logo" color-interpolation-filters="sRGB"><feColorMatrix type="matrix" values="0 0 0 0 1 0 0 0 0 1 0 0 0 0 1 -0.2126 -0.7152 -0.0722 0 1"/><feComposite in2="SourceAlpha" operator="in"/></filter></defs>
<style>
text { fill: #ffffff; }
[stroke]:not([stroke="none"]) { stroke: #ffffff; }
[fill]:not([fill="none"]) { fill: #ffffff; }
svg > rect[width][height] { fill: #143e70; }
image { filter: url(#blueprint-logo); }
</style>
<rect class="blueprint-paper" width="${viewBox[2]}" height="${viewBox[3]}" fill="#143e70"/>
<!-- blueprint-presentation:end -->\n`;
  return svg.replace(/(<svg\b[^>]*>\n?)/,`$1${presentation}`);
}
if(process.argv[1]&&resolve(process.argv[1])===fileURLToPath(import.meta.url)){
  const paths=process.argv.slice(2);if(!paths.length)throw Error('Usage: website-blueprint.mjs DRAWING.svg ...');
  for(const path of paths){writeFileSync(path,blueprint(readFileSync(path,'utf8')));console.log(`Blueprint: ${path}`);}
}
