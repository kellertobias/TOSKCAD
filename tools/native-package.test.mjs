import assert from 'node:assert/strict';
import {mkdtempSync,mkdirSync,readFileSync,rmSync,writeFileSync,chmodSync,statSync,copyFileSync} from 'node:fs';
import {execFileSync,spawnSync} from 'node:child_process';
import {join} from 'node:path';
import {tmpdir} from 'node:os';
import test from 'node:test';
import {assembleRuntime,checkProjectContainer,collectNotices,isNoticeFile,nativeSourceHash,peImports,runtimeExecutable,validateExamples} from './native-package.mjs';
import {readdirSync} from 'node:fs';
import {createHash} from 'node:crypto';

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
      const desktop=readFileSync(join(stage,'multicad.desktop'),'utf8');
      assert.match(desktop,/^Icon=multicad$/m);
      assert.match(desktop,/^Exec=multicad %f$/m);
      assert.match(desktop,/^MimeType=application\/x-toskcad-project;application\/x-multicad-project;$/m);
      const mime=readFileSync(join(stage,'share/mime/packages/toskcad.xml'),'utf8');
      assert.match(mime,/<glob pattern="\*\.tcd" case-sensitive="false"\/>/);
      assert.match(mime,/<glob pattern="\*\.multicad" case-sensitive="false"\/>/);
      assert.deepEqual(readFileSync(join(stage,'share/icons/hicolor/512x512/apps/multicad.png')),readFileSync(new URL('../assets/branding/app/multicad.png',import.meta.url)));
    }
    assert(readFileSync(join(layout.resources,'OCCT-LGPL-EXCEPTION.txt'),'utf8').length>100);
    const cantarell=readFileSync(join(layout.resources,'third-party/bundled-fonts/Cantarell-Regular.txt'),'utf8');
    assert.match(cantarell,/SIL OPEN FONT LICENSE Version 1\.1/);
    assert.match(cantarell,/Jakub Steiner/);
    const readme=readFileSync(join(stage,'README.txt'),'utf8');
    assert.match(readme,/constrained sketches and initial native solid extrusion/);
    assert.match(readme,/Native \.tcd projects support save\/open/);
    assert.match(readme,/existing \.multicad projects still open and save in place/);
    assert.match(readme,/workflows remain under development/);
    assert.match(readme,/multicad --mcp serves a local MCP client over stdin\/stdout; it opens no network port/);
    assert.doesNotMatch(readme,/project format.*not implemented|CAD workspaces.*not implemented|unsigned native development host/);
    // Every listed demo ships byte-identical, with its manifest, in the
    // platform's resources folder; nothing else from examples/ is copied.
    const examples=join(layout.resources,'examples');
    const manifest=JSON.parse(readFileSync(new URL('../examples/examples.json',import.meta.url),'utf8'));
    const shipped=['examples.json',...new Set(manifest.examples.flatMap(e=>[e.file,...(e.requires??[])]))];
    assert.deepEqual(readdirSync(examples).sort(),[...new Set(shipped.map(f=>f.split('/')[0]))].sort());
    for(const name of shipped)assert.deepEqual(readFileSync(join(examples,...name.split('/'))),readFileSync(new URL(`../examples/${name}`,import.meta.url)),name);
    if(process.platform==='darwin')assert.equal(layout.resources,join(stage,'MultiCAD.app/Contents/Resources'));
    else assert.equal(layout.resources,join(stage,'resources'));
    assert.match(readme,/Bundled examples \(Examples, or the welcome screen with no recent projects\): 2 m cabinet with doors, drawers and System 32 shelves, Workbench with a welded frame, C-channel front panel, Adjustable drill guide, Apartment with workshop\./);
    assert.match(readme,/editable copy in Documents\/ToskCAD Examples/);
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
    // Document types: .tcd is the current ToskCAD project; .multicad stays openable.
    const info=JSON.parse(execFileSync('plutil',['-convert','json','-o','-',plist],{encoding:'utf8'}));
    const exported=Object.fromEntries(info.UTExportedTypeDeclarations.map(type=>[type.UTTypeIdentifier,type]));
    assert.deepEqual(exported['de.tokenet.toskcad.project'].UTTypeTagSpecification['public.filename-extension'],['tcd']);
    assert.deepEqual(exported['de.tokenet.multicad.project'].UTTypeTagSpecification['public.filename-extension'],['multicad']);
    assert.equal(exported['de.tokenet.toskcad.project'].UTTypeDescription,'ToskCAD Project');
    for(const type of Object.values(exported))assert(type.UTTypeConformsTo.includes('public.data'));
    assert.deepEqual(info.CFBundleDocumentTypes.map(type=>[type.LSItemContentTypes,type.CFBundleTypeRole,type.LSHandlerRank]),
      [[['de.tokenet.toskcad.project'],'Editor','Owner'],[['de.tokenet.multicad.project'],'Editor','Owner']]);
    execFileSync('plutil',['-lint',plist]);
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

// A minimal format-2 container with the given canonical source text.
function container(sourceText,{name='Fixture'}={}){
  const source=Buffer.from(sourceText);
  const manifest=Buffer.from(JSON.stringify({identity:{id:'00000000-0000-4000-8000-000000000001',name},source:{length:source.length,sha256:[...createHash('sha256').update(source).digest()]},required_capabilities:[],payloads:[],caches:[],recovery:null}));
  const header=Buffer.alloc(52);header.write('MCADPRJ\0',0,'latin1');header.writeUInt32LE(2,8);header.writeBigUInt64LE(BigInt(manifest.length),12);
  createHash('sha256').update(manifest).digest().copy(header,20);
  return Buffer.concat([header,manifest,source]);
}
test('package validation lists every shipped demo and refuses missing, unlisted or damaged examples',()=>{
  const shipped=validateExamples();
  assert(shipped.examples.includes('welded-workbench.tcd')&&shipped.examples.includes('front-panel.tcd'));
  for(const project of shipped.projects)assert.equal(project.version,2);
  const directory=mkdtempSync(join(tmpdir(),'multicad-examples-'));
  const manifest=examples=>writeFileSync(join(directory,'examples.json'),JSON.stringify({format:'toskcad.examples',version:1,examples}));
  try{
    writeFileSync(join(directory,'a.tcd'),container('{"name":"A"}'));
    writeFileSync(join(directory,'b.tcd'),container('{"name":"B"}'));
    mkdirSync(join(directory,'library'));writeFileSync(join(directory,'library/rails.tcd'),container('{"name":"Rails"}'));
    manifest([{file:'a.tcd'},{file:'b.tcd',requires:['a.tcd','library/rails.tcd']}]);
    assert.deepEqual(validateExamples(directory).files,['a.tcd','b.tcd','examples.json','library/rails.tcd']);
    manifest([{file:'a.tcd'}]);
    assert.throws(()=>validateExamples(directory),/Add b\.tcd to examples\/examples\.json/);
    manifest([{file:'a.tcd'},{file:'b.tcd'},{file:'c.tcd'}]);
    assert.throws(()=>validateExamples(directory),/c\.tcd is listed .* but missing/);
    manifest([{file:'a.tcd'},{file:'b.tcd',requires:['../outside.tcd']}]);
    assert.throws(()=>validateExamples(directory),/not a relative path/);
    manifest([{file:'a.tcd'},{file:'b.tcd',requires:['library/gone.tcd']}]);
    assert.throws(()=>validateExamples(directory),/requires library\/gone\.tcd, which is missing/);
    manifest([{file:'a.tcd'},{file:'b.tcd'}]);
    const good=readFileSync(join(directory,'a.tcd'));
    const damaged=Buffer.from(good);damaged[damaged.length-2]^=1;writeFileSync(join(directory,'a.tcd'),damaged);
    assert.throws(()=>validateExamples(directory),/a\.tcd: canonical source checksum mismatch/);
    writeFileSync(join(directory,'a.tcd'),good.subarray(0,good.length-3));
    assert.throws(()=>validateExamples(directory),/truncated/);
    writeFileSync(join(directory,'a.tcd'),Buffer.concat([good,Buffer.from('x')]));
    assert.throws(()=>validateExamples(directory),/trailing data/);
    writeFileSync(join(directory,'a.tcd'),'{"not":"native"}');
    assert.throws(()=>validateExamples(directory),/not a native ToskCAD project/);
    writeFileSync(join(directory,'a.tcd'),container('{"path":"/Users/someone/cad/rail.tcd"}'));
    assert.throws(()=>checkProjectContainer(join(directory,'a.tcd')),/absolute developer path/);
    writeFileSync(join(directory,'a.tcd'),good);
    writeFileSync(join(directory,'examples.json'),'{');
    assert.throws(()=>validateExamples(directory),/not JSON/);
    rmSync(join(directory,'examples.json'));
    assert.throws(()=>validateExamples(directory),/Missing .*examples\.json/);
  }finally{rmSync(directory,{recursive:true,force:true});}
});
