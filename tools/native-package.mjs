// Native runtime layout, source/notice closure and packaged smoke checks.
import {execFileSync} from 'node:child_process';
import {cpSync,copyFileSync,existsSync,lstatSync,mkdirSync,readFileSync,readdirSync,realpathSync,writeFileSync} from 'node:fs';
import {dirname,join,resolve,relative,sep} from 'node:path';
import {fileURLToPath} from 'node:url';
import {createHash} from 'node:crypto';

const root=resolve(dirname(fileURLToPath(import.meta.url)),'..');
const run=(command,args,options={})=>execFileSync(command,args,{cwd:root,encoding:'utf8',stdio:['ignore','pipe','pipe'],...options})?.trim()??'';
const escapeXML=value=>value.replaceAll('&','&amp;').replaceAll('<','&lt;').replaceAll('>','&gt;').replaceAll('"','&quot;');
export function nativeSourceHash(source){
  const paths=[];walk(source,path=>paths.push(path));
  // Rust Path ordering compares components (a/child precedes a.cpp), not the
  // flattened path string. Keep source identity identical to the build helper.
  paths.sort((a,b)=>{const aa=a.split(sep),bb=b.split(sep);for(let i=0;i<Math.min(aa.length,bb.length);i++){const order=Buffer.compare(Buffer.from(aa[i]),Buffer.from(bb[i]));if(order)return order;}return aa.length-bb.length;});
  const hash=createHash('sha256');
  for(const path of paths){hash.update(relative(source,path).split(sep).join('/'));hash.update(Buffer.from([0]));hash.update(createHash('sha256').update(readFileSync(path)).digest());}
  return hash.digest('hex');
}

// Native project files: .tcd for new projects; .multicad stays openable. Both
// name the same container, so both document types open in ToskCAD as Editor.
export const PROJECT_TYPES=[
  {uti:'de.tokenet.toskcad.project',mime:'application/x-toskcad-project',extension:'tcd',name:'ToskCAD Project'},
  {uti:'de.tokenet.multicad.project',mime:'application/x-multicad-project',extension:'multicad',name:'ToskCAD Project (legacy .multicad)'},
];
function macOSDocumentTypes(){
  const strings=values=>`<array>${values.map(value=>`<string>${escapeXML(value)}</string>`).join('')}</array>`;
  const documents=PROJECT_TYPES.map(type=>`<dict><key>CFBundleTypeName</key><string>${escapeXML(type.name)}</string><key>CFBundleTypeRole</key><string>Editor</string><key>LSHandlerRank</key><string>Owner</string><key>CFBundleTypeIconFile</key><string>AppIcon.icns</string><key>LSItemContentTypes</key>${strings([type.uti])}</dict>`).join('');
  const exported=PROJECT_TYPES.map(type=>`<dict><key>UTTypeIdentifier</key><string>${type.uti}</string><key>UTTypeDescription</key><string>${escapeXML(type.name)}</string><key>UTTypeConformsTo</key>${strings(['public.data','public.content'])}<key>UTTypeIconFile</key><string>AppIcon.icns</string><key>UTTypeTagSpecification</key><dict><key>public.filename-extension</key>${strings([type.extension])}<key>public.mime-type</key>${strings([type.mime])}</dict></dict>`).join('');
  return `<key>CFBundleDocumentTypes</key><array>${documents}</array><key>UTExportedTypeDeclarations</key><array>${exported}</array>`;
}
// Bundled example projects (CAD-186): examples/examples.json lists every
// top-level .tcd demo and the further files each requires. Packaging copies
// exactly that closure into Resources/examples (macOS) or resources/examples
// (Windows/Linux) after checking each native container's envelope.
export const EXAMPLES_FORMAT='toskcad.examples';
const DEVELOPER_PATH=/(?:\/(?:Users|home)\/[^"\/\\]+\/|[A-Za-z]:\\\\Users\\\\|\/private\/var\/folders\/)/;
const relativeExamplePath=value=>typeof value==='string'&&value.length>0&&!value.includes('\\')&&!value.startsWith('/')&&value.split('/').every(part=>part&&part!=='.'&&part!=='..');
const hashOf=bytes=>createHash('sha256').update(bytes).digest();
const sameHash=(digest,value)=>Array.isArray(value)&&value.length===32&&Buffer.from(value).equals(digest);
export function checkProjectContainer(path){
  const name=path.split(/[\\/]/).at(-1);
  const fail=reason=>{throw new Error(`Example ${name}: ${reason}. Regenerate it with its generator (docs/architecture/demo-projects.md).`);};
  const bytes=readFileSync(path);
  if(bytes.length<52||!bytes.subarray(0,8).equals(Buffer.from('MCADPRJ\0','latin1')))fail('not a native ToskCAD project');
  const version=bytes.readUInt32LE(8);if(version!==1&&version!==2)fail(`unsupported format version ${version}`);
  const length=Number(bytes.readBigUInt64LE(12));if(length>4*1024*1024||52+length>bytes.length)fail('manifest length exceeds the file');
  const manifestBytes=bytes.subarray(52,52+length);
  if(!hashOf(manifestBytes).equals(bytes.subarray(20,52)))fail('manifest checksum mismatch (damaged file)');
  let manifest;try{manifest=JSON.parse(manifestBytes.toString('utf8'));}catch{fail('manifest is not JSON');}
  if(typeof manifest.identity?.name!=='string'||!manifest.identity.name)fail('missing project identity');
  if(manifest.recovery)fail('is a recovery snapshot, not a project');
  let offset=52+length,source;
  if(version===2){
    const chunk=manifest.source;if(!chunk||!Number.isSafeInteger(chunk.length))fail('missing canonical source record');
    source=bytes.subarray(offset,offset+chunk.length);if(source.length!==chunk.length)fail('canonical source is truncated');
    if(!sameHash(hashOf(source),chunk.sha256))fail('canonical source checksum mismatch (damaged file)');
    offset+=chunk.length;
    for(const record of [...(manifest.payloads??[]),...(manifest.caches??[])]){
      const part=bytes.subarray(offset,offset+record.chunk.length);
      if(part.length!==record.chunk.length||!sameHash(hashOf(part),record.chunk.sha256))fail('embedded payload is damaged or truncated');
      offset+=record.chunk.length;
    }
    if(offset!==bytes.length)fail('unexpected trailing data');
  }else source=Buffer.from(JSON.stringify(manifest.document??null));
  const text=source.toString('utf8');
  try{JSON.parse(text);}catch{fail('canonical source is not JSON');}
  const developer=DEVELOPER_PATH.exec(text)||DEVELOPER_PATH.exec(manifestBytes.toString('utf8'));
  if(developer)fail(`contains the absolute developer path "${developer[0]}"`);
  return {name:manifest.identity.name,version,document:manifest.identity.id};
}
export function validateExamples(directory=join(root,'examples')){
  const manifestPath=join(directory,'examples.json');
  if(!existsSync(manifestPath))throw new Error(`Missing ${manifestPath}: list every bundled example there (docs/architecture/demo-projects.md)`);
  let manifest;try{manifest=JSON.parse(readFileSync(manifestPath,'utf8'));}catch(error){throw new Error(`examples/examples.json is not JSON: ${error.message}`);}
  if(manifest?.format!==EXAMPLES_FORMAT||manifest.version!==1||!Array.isArray(manifest.examples))throw new Error(`examples/examples.json must have format "${EXAMPLES_FORMAT}", version 1 and an examples array`);
  for(const key of Object.keys(manifest))if(!['format','version','examples'].includes(key))throw new Error(`examples/examples.json: unknown field "${key}"`);
  const listed=[],files=new Set(['examples.json']);
  for(const entry of manifest.examples){
    for(const key of Object.keys(entry??{}))if(!['file','requires'].includes(key))throw new Error(`examples/examples.json: unknown example field "${key}"`);
    const file=entry?.file;
    if(!relativeExamplePath(file)||file.includes('/')||!/\.tcd$/i.test(file))throw new Error(`examples/examples.json: "${file}" must be a .tcd file at the top of examples/`);
    if(listed.includes(file))throw new Error(`examples/examples.json lists ${file} twice`);
    if(!existsSync(join(directory,file)))throw new Error(`Bundled example ${file} is listed in examples/examples.json but missing`);
    listed.push(file);files.add(file);
    for(const required of entry.requires??[]){
      if(!relativeExamplePath(required))throw new Error(`examples/examples.json: ${file} requires "${required}", which is not a relative path inside examples/`);
      if(!existsSync(join(directory,required)))throw new Error(`Bundled example ${file} requires ${required}, which is missing from examples/`);
      files.add(required);
    }
  }
  const unlisted=readdirSync(directory).filter(name=>/\.tcd$/i.test(name)&&!listed.includes(name));
  if(unlisted.length)throw new Error(`Add ${unlisted.join(', ')} to examples/examples.json; every bundled .tcd must be listed`);
  if(!listed.length)throw new Error('examples/examples.json lists no example projects');
  const projects=[...files].filter(file=>/\.tcd$/i.test(file)).map(file=>({file,...checkProjectContainer(join(directory,file))}));
  return {examples:listed,files:[...files].sort(),projects};
}
export function bundleExamples(resources,directory=join(root,'examples')){
  const checked=validateExamples(directory);
  const target=join(resources,'examples');mkdirSync(target,{recursive:true});
  for(const file of checked.files){const destination=join(target,...file.split('/'));mkdirSync(dirname(destination),{recursive:true});copyFileSync(join(directory,...file.split('/')),destination);}
  return checked;
}
export function validateBundledLibrary(directory=join(root,'library')){
  const base=realpathSync(directory),manifest=JSON.parse(readFileSync(join(directory,'library.json'),'utf8'));
  const only=(value,keys)=>Object.keys(value??{}).every(key=>keys.includes(key));
  if(!only(manifest,['format','version','revision','entries','requires'])||manifest.format!=='toskcad.bundled-library'||manifest.version!==1||typeof manifest.revision!=='string'||!manifest.revision||manifest.revision.length>100||!Array.isArray(manifest.entries)||!manifest.entries.length||manifest.entries.length>256||!Array.isArray(manifest.requires)||manifest.requires.length>256)throw new Error('Invalid bundled Library manifest');
  const inside=file=>{
    if(!relativeExamplePath(file)||file.includes(':'))throw new Error(`Invalid bundled Library path: ${file}`);
    const path=join(directory,...file.split('/'));
    if(!lstatSync(path).isFile()||!realpathSync(path).startsWith(base+sep))throw new Error(`Bundled Library path is not a regular in-folder file: ${file}`);
    return path;
  };
  const files=new Set(['library.json']),identities=new Set(),projects=[];
  for(const entry of manifest.entries){
    if(!only(entry,['file','document','sha256'])||typeof entry.file!=='string'||entry.file.includes('/')||!entry.file.endsWith('.tcd')||files.has(entry.file)||typeof entry.document!=='string'||identities.has(entry.document)||! /^[0-9a-f]{64}$/.test(entry.sha256??''))throw new Error('Invalid or duplicate bundled Library entry');
    const path=inside(entry.file),bytes=readFileSync(path);
    if(bytes.length>32*1024*1024||createHash('sha256').update(bytes).digest('hex')!==entry.sha256)throw new Error(`Bundled Library checksum mismatch: ${entry.file}`);
    const project=checkProjectContainer(path);
    if(project.document!==entry.document)throw new Error(`Bundled Library document identity mismatch: ${entry.file}`);
    projects.push({file:entry.file,...project});files.add(entry.file);identities.add(entry.document);
  }
  for(const required of manifest.requires){inside(required);files.add(required);}
  for(const required of ['curation.json','washers-curation.json','README.md','sources/bolts/bearings.blt','sources/bolts/washer.blt','sources/bolts/WASHER-NOTICES.md','machine-screws-curation.json','sources/bolts/hex_socket.blt','sources/bolts/hex.blt','sources/bolts/MACHINE-SCREW-NOTICES.md','sources/bolts/COPYING.LESSER'])if(!files.has(required))throw new Error(`Bundled Library notice/source closure is missing ${required}`);
  const curation=JSON.parse(readFileSync(inside('curation.json'),'utf8'));
  if(curation.data_license!=='LGPL-2.1-or-later'||createHash('sha256').update(readFileSync(inside('sources/bolts/bearings.blt'))).digest('hex')!==curation.data_sha256)throw new Error('Bundled Library reference license/hash differs from curation');
  const washers=JSON.parse(readFileSync(inside('washers-curation.json'),'utf8'));
  if(washers.format!=='toskcad.washer-curation'||washers.version!==1||washers.data_license!=='LGPL-2.1-or-later'||createHash('sha256').update(readFileSync(inside('sources/bolts/washer.blt'))).digest('hex')!==washers.data_sha256)throw new Error('Bundled Library washer reference license/hash differs from curation');
  const screws=JSON.parse(readFileSync(inside('machine-screws-curation.json'),'utf8'));
  if(screws.format!=='toskcad.machine-screw-curation'||screws.version!==1||!Array.isArray(screws.data_sources)||screws.data_sources.length!==2)throw new Error('Invalid bundled machine screw curation');
  for(const source of screws.data_sources)if(!files.has(source.file)||source.license!=='LGPL-2.1-or-later'||createHash('sha256').update(readFileSync(inside(source.file))).digest('hex')!==source.sha256)throw new Error('Bundled Library machine screw source license/hash differs from curation');
  const unlisted=readdirSync(directory).filter(name=>name.endsWith('.tcd')&&!files.has(name));
  if(unlisted.length)throw new Error(`Unlisted bundled Library projects: ${unlisted.join(', ')}`);
  return {files:[...files].sort(),projects};
}
export function bundleLibrary(resources,directory=join(root,'library')){
  const checked=validateBundledLibrary(directory),target=join(resources,'library');mkdirSync(target,{recursive:true});
  for(const file of checked.files){const destination=join(target,...file.split('/'));mkdirSync(dirname(destination),{recursive:true});copyFileSync(join(directory,...file.split('/')),destination);}
  return checked;
}
export function runtimeExecutable(stage,platform=process.platform){return platform==='darwin'?join(stage,'MultiCAD.app/Contents/MacOS/multicad'):join(stage,platform==='win32'?'multicad.exe':'multicad');}
// The current development/archive policy is ad-hoc signing. Seal the outer
// bundle after its resources; a copied executable signature does not seal a
// newly assembled app. Publisher signing must follow the final resource write.
function sealMacOSBundle(resources){
  if(process.platform!=='darwin')return;
  const bundle=resolve(resources,'../..');
  run('codesign',['--force','--sign','-',bundle]);
  run('codesign',['--verify','--deep','--strict',bundle]);
}
export function assembleRuntime(stage,binary,version){
  mkdirSync(stage,{recursive:true});
  const executable=runtimeExecutable(stage);mkdirSync(dirname(executable),{recursive:true});copyFileSync(binary,executable);
  if(process.platform==='darwin'){
    const contents=join(stage,'MultiCAD.app/Contents');
    writeFileSync(join(contents,'Info.plist'),`<?xml version="1.0" encoding="UTF-8"?><!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd"><plist version="1.0"><dict><key>CFBundleExecutable</key><string>multicad</string><key>CFBundleIdentifier</key><string>de.tokenet.multicad</string><key>CFBundleName</key><string>ToskCAD</string><key>CFBundleDisplayName</key><string>ToskCAD</string><key>CFBundleIconFile</key><string>AppIcon.icns</string><key>CFBundlePackageType</key><string>APPL</string><key>NSPrincipalClass</key><string>NSApplication</string><key>CFBundleShortVersionString</key><string>${escapeXML(version.split('-')[0])}</string><key>CFBundleVersion</key><string>${escapeXML(version.split('-')[0])}</string><key>LSMinimumSystemVersion</key><string>13.0</string><key>NSHighResolutionCapable</key><true/>${macOSDocumentTypes()}</dict></plist>\n`);
  }else if(process.platform==='linux'){
    writeFileSync(join(stage,'multicad.desktop'),`[Desktop Entry]\nType=Application\nName=ToskCAD\nExec=multicad %f\nIcon=multicad\nTerminal=false\nCategories=Graphics;Engineering;\nMimeType=${PROJECT_TYPES.map(type=>type.mime).join(';')};\n`);
    const mime=join(stage,'share/mime/packages');mkdirSync(mime,{recursive:true});
    writeFileSync(join(mime,'toskcad.xml'),`<?xml version="1.0" encoding="UTF-8"?>\n<mime-info xmlns="http://www.freedesktop.org/standards/shared-mime-info">\n${PROJECT_TYPES.map(type=>`  <mime-type type="${type.mime}">\n    <comment>${escapeXML(type.name)}</comment>\n    <glob pattern="*.${type.extension}" case-sensitive="false"/>\n  </mime-type>\n`).join('')}</mime-info>\n`);
    const icons=join(stage,'share/icons/hicolor/512x512/apps');mkdirSync(icons,{recursive:true});
    copyFileSync(join(root,'assets/branding/app/multicad.png'),join(icons,'multicad.png'));
  }
  const resources=process.platform==='darwin'?join(stage,'MultiCAD.app/Contents/Resources'):join(stage,'resources');mkdirSync(resources,{recursive:true});
  const iconName=process.platform==='darwin'?'AppIcon.icns':process.platform==='win32'?'AppIcon.ico':'AppIcon.png';
  copyFileSync(join(root,'assets/branding/app',iconName),join(resources,iconName));
  copyFileSync(join(root,'LICENSE'),join(resources,'LICENSE'));
  copyFileSync(join(root,'native-dependencies.json'),join(resources,'native-dependencies.json'));
  for(const name of ['OCCT-LICENSE-LGPL-2.1.txt','OCCT-LGPL-EXCEPTION.txt'])copyFileSync(join(root,'docs/architecture/foundation',name),join(resources,name));
  // Fonts vendored in the source tree for sketch text ship their licence texts.
  const fonts=join(root,'crates/multicad-core/fonts'),fontNotices=join(resources,'third-party/bundled-fonts');mkdirSync(fontNotices,{recursive:true});
  for(const name of readdirSync(fonts))if(isNoticeFile(join(fonts,name)))copyFileSync(join(fonts,name),join(fontNotices,name));
  bundleLibrary(resources);
  const examples=bundleExamples(resources);
  writeFileSync(join(stage,'README.txt'),`ToskCAD ${version} native development build.\nIncludes parameter editing, constrained sketches and initial native solid extrusion.\nNative .tcd projects support save/open, embedded resources and explicit recovery; existing .multicad projects still open and save in place.\nSpecialized workspaces and drawing and 3D export workflows remain under development.\nBundled examples (Examples, or the welcome screen with no recent projects): ${examples.projects.filter(p=>examples.examples.includes(p.file)).map(p=>p.name).join(', ')}. Opening one saves an editable copy in Documents/ToskCAD Examples; the bundled originals are never changed.\nRun with no arguments to open the native window, or pass one PROJECT.tcd to open it; --diagnostics is headless.\nmulticad --mcp serves a local MCP client over stdin/stdout; it opens no network port.\nmacOS: open MultiCAD.app. Windows: run multicad.exe. Linux: run ./multicad on an Ubuntu 24.04+ desktop with graphics drivers.\nNo Python, Node.js, FreeCAD or source checkout is required at runtime.\n${process.platform==='darwin'?'This macOS bundle is ad-hoc signed, without publisher signing or notarization.':'This development build is not publisher-signed.'}\nSource and third-party notices accompany release archives.\n`);
  sealMacOSBundle(resources);
  return {executable,resources};
}

export function createSourceTree(destination,commit,version){
  mkdirSync(destination,{recursive:true});
  const archive=execFileSync('git',['archive','--format=tar',commit],{cwd:root,maxBuffer:128*1024*1024});
  execFileSync('tar',['-xf','-','-C',destination],{input:archive});
  // Everything needed to rebuild/relink the static native library is supplied.
  const native=join(destination,'artifacts/native-deps');mkdirSync(native,{recursive:true});
  copyFileSync(join(root,'artifacts/native-deps/occt-7.9.3.tar.gz'),join(native,'occt-7.9.3.tar.gz'));
  // Include actual extracted source too, so any local native patches remain
  // part of the corresponding source rather than only shipping the base tar.
  cpSync(join(root,'artifacts/native-deps/OCCT-7_9_3'),join(native,'OCCT-7_9_3'),{recursive:true});
  const vendorConfig=run('cargo',['vendor','--offline','--locked','--versioned-dirs',join(destination,'vendor')],{cwd:destination,maxBuffer:16*1024*1024});
  const cargoDirectory=join(destination,'.cargo');mkdirSync(cargoDirectory,{recursive:true});
  const config=join(cargoDirectory,'config.toml');
  const previous=existsSync(config)?readFileSync(config,'utf8'):'';
  const local=vendorConfig.replace(/^directory = .*$/m,'directory = "vendor"');
  writeFileSync(config,previous+'\n'+local+'\n');
  writeFileSync(join(destination,'SOURCE-REPRODUCTION.md'),`# MultiCAD ${version}\n\nSource commit: ${commit}\n\nThis tree includes product source, Cargo.lock, vendored Cargo dependencies and the checksum-pinned OCCT source archive. It permits rebuilding the complete application against a changed OCCT library.\n\nInstall the pinned Rust toolchain, CMake, a C++17 toolchain and Node.js (build tooling only). From this folder run:\n\n    node tools/foundation-deps.mjs\n    MULTICAD_RELEASE_VERSION=${version} MULTICAD_BUILD_COMMIT=${commit} cargo build --workspace --release --locked --offline\n\nOn PowerShell set the two environment variables separately before cargo. Native caches and target output can be deleted and regenerated; no preinstalled OCCT/FreeCAD/Python is used. The upstream OCCT archive and exception/license are included. To change OCCT, replace the source/archive pin and rebuild; do not use the old native cache as evidence of a changed library. Build tools and OS SDKs are prerequisites.\n`);
  const metadata=JSON.parse(run('cargo',['metadata','--offline','--locked','--format-version','1'],{cwd:destination,maxBuffer:32*1024*1024}));
  return metadata;
}
function walk(path,visit){for(const e of readdirSync(path,{withFileTypes:true})){const p=join(path,e.name);if(e.isDirectory())walk(p,visit);else if(e.isFile())visit(p);}}
// Licence texts, including a font's same-stem .txt licence sidecar (Hack-Regular.txt
// beside Hack-Regular.ttf in epaint_default_fonts, used for sketch text outlines).
export function isNoticeFile(path){
  const name=path.split(/[\\/]/).at(-1);
  if(path.endsWith('.rs'))return false;
  if(/licen[cs]e|copying|copyright|notice|\bofl\b|\bufl\b/i.test(name))return true;
  const font=/^(.+)\.txt$/i.exec(name);
  return Boolean(font)&&['.ttf','.otf'].some(extension=>existsSync(join(dirname(path),font[1]+extension)));
}
export function collectNotices(resources,source,metadata){
  const notices=join(resources,'third-party');mkdirSync(notices,{recursive:true});
  const records=[];
  for(const p of metadata.packages){
    if(p.source===null)continue;
    if(!p.license)throw new Error(`Missing license metadata for ${p.name}`);
    const directory=dirname(p.manifest_path);const target=join(notices,`${p.name}-${p.version}`);
    walk(directory,path=>{if(isNoticeFile(path)){const dest=join(target,relative(directory,path));mkdirSync(dirname(dest),{recursive:true});copyFileSync(path,dest);}});
    records.push({name:p.name,version:p.version,license:p.license,source:p.source});
  }
  writeFileSync(join(resources,'third-party.json'),JSON.stringify(records.sort((a,b)=>a.name.localeCompare(b.name)),null,2)+'\n');
  sealMacOSBundle(resources);
}
export function auditRuntime(binary){
  if(process.platform==='darwin'){
    const libraries=run('otool',['-L',binary]).split('\n').slice(1).map(line=>line.trim().split(' (')[0]).filter(Boolean);
    for(const library of libraries)if(!library.startsWith('/System/Library/')&&!library.startsWith('/usr/lib/'))throw new Error(`Unbundled non-system dependency: ${library}`);
    return libraries;
  }
  if(process.platform==='linux'){
    const output=run('ldd',[binary]);if(output.includes('not found'))throw new Error('Unresolved native runtime dependency');
    if(/\/home\/|\/opt\/|\/target\//.test(output))throw new Error('Developer-path runtime dependency');return output.split('\n');
  }
  const imports=peImports(binary);
  for(const name of imports)if(!/^(?:api-ms-|ext-ms-)/i.test(name)&&!existsSync(join(process.env.SystemRoot??'C:\\Windows','System32',name)))throw new Error(`Unbundled non-system Windows dependency: ${name}`);
  return imports;
}
export function peImports(binary){
  const b=readFileSync(binary);const pe=b.readUInt32LE(0x3c);if(b.toString('ascii',pe,pe+4)!=='PE\0\0')throw new Error('Not a PE executable');
  const optional=pe+24,sections=b.readUInt16LE(pe+6),sectionStart=optional+b.readUInt16LE(pe+20);
  if(b.readUInt16LE(optional)!==0x20b)throw new Error('Expected a 64-bit Windows executable');
  const offset=rva=>{for(let i=0;i<sections;i++){const s=sectionStart+i*40;const start=b.readUInt32LE(s+12),size=Math.max(b.readUInt32LE(s+8),b.readUInt32LE(s+16));if(rva>=start&&rva<start+size)return b.readUInt32LE(s+20)+rva-start;}throw new Error('Unmapped PE RVA');};
  const rva=b.readUInt32LE(optional+120);if(!rva)return [];
  const imports=[];for(let d=offset(rva);b.readUInt32LE(d)||b.readUInt32LE(d+12);d+=20){const start=offset(b.readUInt32LE(d+12)),end=b.indexOf(0,start);if(end<start)throw new Error('Invalid PE import');imports.push(b.toString('ascii',start,end));}
  const forbidden=imports.filter(name=>/^(?:msvcp|vcruntime|concrt|python|tk|qt)/i.test(name));
  if(forbidden.length)throw new Error(`Unbundled Windows runtime: ${forbidden.join(', ')}`);
  return imports;
}
