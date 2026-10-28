// Native runtime layout, source/notice closure and packaged smoke checks.
import {execFileSync} from 'node:child_process';
import {cpSync,copyFileSync,existsSync,mkdirSync,readFileSync,readdirSync,writeFileSync} from 'node:fs';
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
    writeFileSync(join(contents,'Info.plist'),`<?xml version="1.0" encoding="UTF-8"?><!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd"><plist version="1.0"><dict><key>CFBundleExecutable</key><string>multicad</string><key>CFBundleIdentifier</key><string>de.tokenet.multicad</string><key>CFBundleName</key><string>ToskCAD</string><key>CFBundleDisplayName</key><string>ToskCAD</string><key>CFBundleIconFile</key><string>AppIcon.icns</string><key>CFBundlePackageType</key><string>APPL</string><key>NSPrincipalClass</key><string>NSApplication</string><key>CFBundleShortVersionString</key><string>${escapeXML(version.split('-')[0])}</string><key>CFBundleVersion</key><string>${escapeXML(version.split('-')[0])}</string><key>LSMinimumSystemVersion</key><string>13.0</string><key>NSHighResolutionCapable</key><true/></dict></plist>\n`);
  }else if(process.platform==='linux'){
    writeFileSync(join(stage,'multicad.desktop'),'[Desktop Entry]\nType=Application\nName=ToskCAD\nExec=multicad\nIcon=multicad\nTerminal=false\nCategories=Graphics;Engineering;\n');
    const icons=join(stage,'share/icons/hicolor/512x512/apps');mkdirSync(icons,{recursive:true});
    copyFileSync(join(root,'assets/branding/app/multicad.png'),join(icons,'multicad.png'));
  }
  const resources=process.platform==='darwin'?join(stage,'MultiCAD.app/Contents/Resources'):join(stage,'resources');mkdirSync(resources,{recursive:true});
  const iconName=process.platform==='darwin'?'AppIcon.icns':process.platform==='win32'?'AppIcon.ico':'AppIcon.png';
  copyFileSync(join(root,'assets/branding/app',iconName),join(resources,iconName));
  copyFileSync(join(root,'LICENSE'),join(resources,'LICENSE'));
  copyFileSync(join(root,'native-dependencies.json'),join(resources,'native-dependencies.json'));
  for(const name of ['OCCT-LICENSE-LGPL-2.1.txt','OCCT-LGPL-EXCEPTION.txt'])copyFileSync(join(root,'docs/architecture/foundation',name),join(resources,name));
  writeFileSync(join(stage,'README.txt'),`ToskCAD ${version} native development build.\nIncludes parameter editing, constrained sketches and initial native solid extrusion.\nNative .multicad projects support save/open, embedded resources and explicit recovery.\nSpecialized workspaces, drawing and 3D export workflows, and the MCP server remain under development.\nRun with no arguments to open the native window; --diagnostics is headless.\nmacOS: open MultiCAD.app. Windows: run multicad.exe. Linux: run ./multicad on an Ubuntu 24.04+ desktop with graphics drivers.\nNo Python, Node.js, FreeCAD or source checkout is required at runtime.\n${process.platform==='darwin'?'This macOS bundle is ad-hoc signed, without publisher signing or notarization.':'This development build is not publisher-signed.'}\nSource and third-party notices accompany release archives.\n`);
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
export function collectNotices(resources,source,metadata){
  const notices=join(resources,'third-party');mkdirSync(notices,{recursive:true});
  const records=[];
  for(const p of metadata.packages){
    if(p.source===null)continue;
    if(!p.license)throw new Error(`Missing license metadata for ${p.name}`);
    const directory=dirname(p.manifest_path);const target=join(notices,`${p.name}-${p.version}`);
    walk(directory,path=>{const name=path.split(/[\\/]/).at(-1);if(/licen[cs]e|copying|copyright|notice|\bofl\b/i.test(name)&&!path.endsWith('.rs')){const dest=join(target,relative(directory,path));mkdirSync(dirname(dest),{recursive:true});copyFileSync(path,dest);}});
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
