import assert from 'node:assert/strict';
import {mkdtempSync,mkdirSync,readFileSync,rmSync,writeFileSync,chmodSync,statSync,copyFileSync} from 'node:fs';
import {execFileSync,spawnSync} from 'node:child_process';
import {join} from 'node:path';
import {tmpdir} from 'node:os';
import test from 'node:test';
import {assembleRuntime,collectNotices,isNoticeFile,nativeSourceHash,peImports,runtimeExecutable} from './native-package.mjs';

// Use a real Mach-O to exercise native executable signing. This fixture never
// creates a window and is not an application runtime or a shipped product.
function executableFixture(directory){
  const binary=join(directory,'fixture');
  if(process.platform==='darwin')execFileSync('xcrun',['clang','-x','c','-o',binary,'-'],{input:'int main(void) { return 0; }\n'});
  else{writeFileSync(binary,'layout fixture only');chmodSync(binary,0o755);}
  return binary;
}

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
    for(const name of ['a','b']){mkdirSync(join(directory,name,'nested'),{recursive:true});writeFileSync(join(directory,name,'nested/source.cpp'),'source');writeFileSync(join(directory,name,'nested.cpp'),'flat');}
    assert.equal(nativeSourceHash(join(directory,'a')),'dedfba9092ed6c79fa868edb60508eb5373cf77e22910d45ee6368b770075067');
    assert.equal(nativeSourceHash(join(directory,'a')),nativeSourceHash(join(directory,'b')));
    writeFileSync(join(directory,'b/nested/source.cpp'),'edited source');
    assert.notEqual(nativeSourceHash(join(directory,'a')),nativeSourceHash(join(directory,'b')));
  }finally{rmSync(directory,{recursive:true,force:true});}
});
test('native development layout includes a copied executable, identity pin and notices',()=>{
  const directory=mkdtempSync(join(tmpdir(),'multicad-layout-'));
  try{
    const binary=executableFixture(directory),original=readFileSync(binary);
    const stage=join(directory,'stage');const layout=assembleRuntime(stage,binary,'1.2.3-dev');
    assert.deepEqual(readFileSync(binary),original,'Packaging must never sign or modify the compiler output');
    if(process.platform!=='darwin')assert.deepEqual(readFileSync(runtimeExecutable(stage)),original);
    assert.equal(JSON.parse(readFileSync(join(layout.resources,'native-dependencies.json'),'utf8')).occt.version,'7.9.3');
    const iconName=process.platform==='darwin'?'AppIcon.icns':process.platform==='win32'?'AppIcon.ico':'AppIcon.png';
    assert.deepEqual(readFileSync(join(layout.resources,iconName)),readFileSync(new URL(`../assets/branding/app/${iconName}`,import.meta.url)));
    if(process.platform==='linux'){
      assert.match(readFileSync(join(stage,'multicad.desktop'),'utf8'),/^Icon=multicad$/m);
      assert.deepEqual(readFileSync(join(stage,'share/icons/hicolor/512x512/apps/multicad.png')),readFileSync(new URL('../assets/branding/app/multicad.png',import.meta.url)));
    }
    assert(readFileSync(join(layout.resources,'OCCT-LGPL-EXCEPTION.txt'),'utf8').length>100);
    const readme=readFileSync(join(stage,'README.txt'),'utf8');
    assert.match(readme,/constrained sketches and initial native solid extrusion/);
    assert.match(readme,/Native \.multicad projects support save\/open/);
    assert.match(readme,/workflows remain under development/);
    assert.match(readme,/multicad --mcp serves a local MCP client over stdin\/stdout; it opens no network port/);
    assert.doesNotMatch(readme,/project format.*not implemented|CAD workspaces.*not implemented|unsigned native development host/);
    if(process.platform!=='win32')assert(statSync(layout.executable).mode&0o111);
  }finally{rmSync(directory,{recursive:true,force:true});}
});
test('macOS development bundle declares its principal class and seals every resource',{skip:process.platform!=='darwin'},()=>{
  const directory=mkdtempSync(join(tmpdir(),'multicad-macos-seal-'));
  try{
    const stage=join(directory,'stage'),layout=assembleRuntime(stage,executableFixture(directory),'1.2.3-dev');
    const bundle=join(stage,'MultiCAD.app');
    const plist=join(bundle,'Contents/Info.plist');
    assert.equal(execFileSync('plutil',['-extract','NSPrincipalClass','raw','-o','-',plist],{encoding:'utf8'}).trim(),'NSApplication');
    assert.equal(execFileSync('plutil',['-extract','CFBundleIconFile','raw','-o','-',plist],{encoding:'utf8'}).trim(),'AppIcon.icns');
    assert.equal(execFileSync('plutil',['-extract','CFBundleShortVersionString','raw','-o','-',plist],{encoding:'utf8'}).trim(),'1.2.3');
    execFileSync('codesign',['--verify','--deep','--strict',bundle]);
    const signature=spawnSync('codesign',['--display','--verbose=2',bundle],{encoding:'utf8'});
    assert.equal(signature.status,0,signature.stderr);
    assert.match(signature.stderr,/Signature=adhoc/);
    // Notice collection changes Resources after development assembly. It must
    // reseal the final closure before release packaging, using the same policy.
    const vendor=join(directory,'vendor-fixture');mkdirSync(vendor);
    writeFileSync(join(vendor,'LICENSE'),'Third-party notice fixture');
    collectNotices(layout.resources,directory,{packages:[{source:'registry+https://example.invalid',name:'notice-fixture',version:'1.0.0',license:'MIT',manifest_path:join(vendor,'Cargo.toml')}]});
    assert.deepEqual(JSON.parse(readFileSync(join(layout.resources,'third-party.json'),'utf8')),[{name:'notice-fixture',version:'1.0.0',license:'MIT',source:'registry+https://example.invalid'}]);
    assert.equal(readFileSync(join(layout.resources,'third-party/notice-fixture-1.0.0/LICENSE'),'utf8'),'Third-party notice fixture');
    execFileSync('codesign',['--verify','--deep','--strict',bundle]);
    writeFileSync(join(layout.resources,'LICENSE'),'tampered resource');
    assert.throws(()=>execFileSync('codesign',['--verify','--deep','--strict',bundle],{stdio:'pipe'}));
    assert.match(readFileSync(join(stage,'README.txt'),'utf8'),/macOS bundle is ad-hoc signed/);
  }finally{rmSync(directory,{recursive:true,force:true});}
});
test('notices include font licence sidecars used by sketch text',()=>{
  const directory=mkdtempSync(join(tmpdir(),'multicad-font-notice-'));
  try{
    for(const name of ['Hack-Regular.ttf','Hack-Regular.txt','UFL.txt','OFL.txt','readme.txt','lib.rs'])writeFileSync(join(directory,name),name);
    assert.equal(isNoticeFile(join(directory,'Hack-Regular.txt')),true);
    assert.equal(isNoticeFile(join(directory,'UFL.txt')),true);
    assert.equal(isNoticeFile(join(directory,'OFL.txt')),true);
    assert.equal(isNoticeFile(join(directory,'readme.txt')),false);
    assert.equal(isNoticeFile(join(directory,'Hack-Regular.ttf')),false);
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
