// Developer-only conversion of the approved artwork; generated assets are committed.
// Run on macOS: node tools/generate-app-icons.mjs
import {execFileSync} from 'node:child_process';
import {mkdirSync,mkdtempSync,readFileSync,rmSync,writeFileSync} from 'node:fs';
import {dirname,join,resolve} from 'node:path';
import {tmpdir} from 'node:os';
import {fileURLToPath} from 'node:url';

if(process.platform!=='darwin')throw new Error('Icon regeneration requires macOS sips and iconutil; other hosts use the committed assets.');
const root=resolve(dirname(fileURLToPath(import.meta.url)),'..');
const source=join(root,'assets/branding/toskcad-icon-blueprint-larger.png');
const output=join(root,'assets/branding/app');mkdirSync(output,{recursive:true});
const temporary=mkdtempSync(join(tmpdir(),'toskcad-icons-'));
const resize=(size,path)=>execFileSync('sips',['-z',String(size),String(size),source,'--out',path],{stdio:'pipe'});
try{
  const iconset=join(temporary,'AppIcon.iconset');mkdirSync(iconset);
  for(const size of [16,32,128,256,512]){
    resize(size,join(iconset,`icon_${size}x${size}.png`));
    resize(size*2,join(iconset,`icon_${size}x${size}@2x.png`));
  }
  execFileSync('iconutil',['-c','icns',iconset,'-o',join(output,'AppIcon.icns')]);
  resize(256,join(output,'AppIcon.png'));
  resize(512,join(output,'multicad.png'));
  // ICO directory entries reference PNG frames (supported by modern Windows).
  const sizes=[16,24,32,48,64,128,256];
  const directory=Buffer.alloc(6+16*sizes.length);directory.writeUInt16LE(1,2);directory.writeUInt16LE(sizes.length,4);
  let offset=directory.length;
  const frames=sizes.map((size,index)=>{
    const path=join(temporary,`${size}.png`);resize(size,path);const bytes=readFileSync(path);
    const entry=6+16*index;directory[entry]=size===256?0:size;directory[entry+1]=directory[entry];
    directory.writeUInt16LE(1,entry+4);directory.writeUInt16LE(32,entry+6);
    directory.writeUInt32LE(bytes.length,entry+8);directory.writeUInt32LE(offset,entry+12);offset+=bytes.length;
    return bytes;
  });
  writeFileSync(join(output,'AppIcon.ico'),Buffer.concat([directory,...frames]));
}finally{rmSync(temporary,{recursive:true,force:true});}
