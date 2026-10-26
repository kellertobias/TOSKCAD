import assert from 'node:assert/strict';
import {mkdtempSync,mkdirSync,readFileSync,rmSync,writeFileSync,chmodSync,statSync,copyFileSync} from 'node:fs';
import {execFileSync} from 'node:child_process';
import {join} from 'node:path';
import {tmpdir} from 'node:os';
import test from 'node:test';
import {assembleRuntime,nativeSourceHash,peImports,runtimeExecutable} from './native-package.mjs';

test('native build entry loads without a pre-existing node_modules directory',()=>{
  const directory=mkdtempSync(join(tmpdir(),'multicad-clean-build-entry-'));
  try{
    mkdirSync(join(directory,'tools'));
    for(const name of ['build.mjs','native-package.mjs'])copyFileSync(new URL(name,import.meta.url),join(directory,'tools',name));
    const output=execFileSync(process.execPath,[join(directory,'tools/build.mjs'),'--help'],{cwd:directory,encoding:'utf8'});
    assert.match(output,/Usage: .\/build/);
  }finally{rmSync(directory,{recursive:true,force:true});}
});

test('native source identity includes content and relative names, not checkout location',()=>{
  const directory=mkdtempSync(join(tmpdir(),'multicad-source-id-'));
  try{
    for(const name of ['a','b']){mkdirSync(join(directory,name,'nested'),{recursive:true});writeFileSync(join(directory,name,'nested/source.cpp'),'source');}
    assert.equal(nativeSourceHash(join(directory,'a')),nativeSourceHash(join(directory,'b')));
    writeFileSync(join(directory,'b/nested/source.cpp'),'edited source');
    assert.notEqual(nativeSourceHash(join(directory,'a')),nativeSourceHash(join(directory,'b')));
  }finally{rmSync(directory,{recursive:true,force:true});}
});
test('native development layout includes a copied executable, identity pin and notices',()=>{
  const directory=mkdtempSync(join(tmpdir(),'multicad-layout-'));
  try{
    const binary=join(directory,'fixture');writeFileSync(binary,'layout fixture only');chmodSync(binary,0o755);
    const stage=join(directory,'stage');const layout=assembleRuntime(stage,binary,'1.2.3-dev');
    assert.equal(readFileSync(runtimeExecutable(stage),'utf8'),'layout fixture only');
    assert.equal(JSON.parse(readFileSync(join(layout.resources,'native-dependencies.json'),'utf8')).occt.version,'7.9.3');
    assert(readFileSync(join(layout.resources,'OCCT-LGPL-EXCEPTION.txt'),'utf8').length>100);
    if(process.platform!=='win32')assert(statSync(layout.executable).mode&0o111);
  }finally{rmSync(directory,{recursive:true,force:true});}
});
function peFixture(name){
  const bytes=Buffer.alloc(0x800);bytes.write('MZ');bytes.writeUInt32LE(0x80,0x3c);bytes.write('PE\0\0',0x80);
  bytes.writeUInt16LE(1,0x86);bytes.writeUInt16LE(0xf0,0x94);bytes.writeUInt16LE(0x20b,0x98);bytes.writeUInt32LE(0x1100,0x98+120);
  bytes.writeUInt32LE(0x600,0x188+8);bytes.writeUInt32LE(0x1000,0x188+12);bytes.writeUInt32LE(0x600,0x188+16);bytes.writeUInt32LE(0x200,0x188+20);
  bytes.writeUInt32LE(0x1200,0x300+12);bytes.write(name+'\0',0x400);return bytes;
}
test('Windows import reader detects a developer runtime dependency',()=>{
  const directory=mkdtempSync(join(tmpdir(),'multicad-pe-contract-'));const file=join(directory,'fixture.exe');
  try{
    writeFileSync(file,peFixture('KERNEL32.dll'));assert.deepEqual(peImports(file),['KERNEL32.dll']);
    writeFileSync(file,peFixture('VCRUNTIME140.dll'));assert.throws(()=>peImports(file),/Unbundled Windows runtime/);
    writeFileSync(file,'invalid');assert.throws(()=>peImports(file));
  }finally{rmSync(directory,{recursive:true,force:true});}
});
