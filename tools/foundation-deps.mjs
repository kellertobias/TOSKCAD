// Developer-only source bootstrap; never imported by the native application.
import { createHash } from 'node:crypto';
import { execFileSync } from 'node:child_process';
import { existsSync, mkdirSync, readFileSync, renameSync, rmSync, writeFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const root=resolve(dirname(fileURLToPath(import.meta.url)),'..');
const directory=join(root,'artifacts/native-deps');
const pin=JSON.parse(readFileSync(join(root,'spikes/native-foundation/native-dependencies.json'),'utf8')).occt;
mkdirSync(directory,{recursive:true});
const archive=join(directory,'occt-7.9.3.tar.gz');
if(!existsSync(archive)) {
  const response=await fetch(pin.url);
  if(!response.ok)throw new Error(`OCCT download failed (${response.status})`);
  const bytes=Buffer.from(await response.arrayBuffer());
  if(createHash('sha256').update(bytes).digest('hex')!==pin.sha256)throw new Error('OCCT archive checksum mismatch');
  const partial=archive+`.${process.pid}.partial`;
  try{writeFileSync(partial,bytes,{flag:'wx'});renameSync(partial,archive);}finally{rmSync(partial,{force:true});}
}
if(createHash('sha256').update(readFileSync(archive)).digest('hex')!==pin.sha256)throw new Error('Cached OCCT archive checksum mismatch');
const source=join(directory,pin.directory);
if(!existsSync(source))execFileSync('tar',['-xzf',archive,'-C',directory],{stdio:'inherit'});
if(!existsSync(join(source,'CMakeLists.txt')))throw new Error('Incomplete OCCT source extraction');
console.log(`OCCT ${pin.version} source ready; ${pin.sha256}`);
