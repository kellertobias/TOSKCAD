import {execFileSync} from 'node:child_process';
import {createHash} from 'node:crypto';
import {existsSync,mkdirSync,mkdtempSync,readFileSync,readdirSync,realpathSync,renameSync,rmSync,writeFileSync} from 'node:fs';
import {dirname,join,resolve} from 'node:path';
import {fileURLToPath,pathToFileURL} from 'node:url';
import {requireCommit,requireVersion} from './release-policy.mjs';
import {assembleRuntime,auditRuntime,collectNotices,createSourceTree,nativeSourceHash,rustCandidateBuildArgs} from './native-package.mjs';
export {assembleRuntime} from './native-package.mjs';

const root=resolve(dirname(fileURLToPath(import.meta.url)),'..');
export const targets=Object.freeze(['linux-x64','macos-arm64','windows-x64']);
export const outputDirectory=join(root,'artifacts/release');
export const rustCandidateDirectory=join(root,'artifacts/rust-candidate');
const kind='native-development-host';
const rustCandidateKind='rust-only-development-host';
const hostTarget=()=>`${{darwin:'macos',win32:'windows',linux:'linux'}[process.platform]}-${process.arch}`;
const run=(command,args,options={})=>execFileSync(command,args,{cwd:root,encoding:'utf8',stdio:['ignore','pipe','pipe'],...options})?.trim()??'';
export const digest=path=>createHash('sha256').update(readFileSync(path)).digest('hex');
export const archiveName=(version,target)=>`multicad-${requireVersion(version)}-${target}.zip`;
export const sourceName=(version,target)=>`multicad-${requireVersion(version)}-${target}-sources.zip`;
export const rustCandidateArchiveName=(version,target)=>`multicad-${requireVersion(version)}-${target}-rust-candidate.zip`;
const rustCandidateSourceName=(version,target)=>`multicad-${requireVersion(version)}-${target}-rust-candidate-sources.zip`;
export function validateDescriptor(value,version,commit,target,archiveHash){
  requireVersion(version);requireCommit(commit);
  if(!targets.includes(target)||value.version!==version||value.tag!==`v${version}`||value.commit!==commit||value.target!==target||value.sha256!==archiveHash||value.archive!==archiveName(version,target)||value.kind!==kind||!/^[a-f0-9]{64}$/.test(value.nativeSourceHash??'')||value.sources?.archive!==sourceName(version,target)||!/^[a-f0-9]{64}$/.test(value.sources?.sha256??''))throw new Error(`Mismatched release asset identity for ${target}`);
}
function verifyArchive(directory,asset,hash){
  const file=join(directory,asset);
  if(digest(file)!==hash||readFileSync(file+'.sha256','utf8')!==`${hash}  ${asset}\n`)throw new Error(`Checksum mismatch for ${asset}`);
  if(readFileSync(file).subarray(0,4).toString('hex')!=='504b0304')throw new Error(`Not a ZIP archive: ${asset}`);
}
export function verifyPackageSet(directory,version,commit){
  requireVersion(version);requireCommit(commit);const allowed=new Set(['release-manifest.json']);
  const assets=targets.map(target=>{
    const name=archiveName(version,target),file=join(directory,name);
    if(!existsSync(file))throw new Error(`Missing required ${target} archive`);
    const descriptor=JSON.parse(readFileSync(file+'.json','utf8'));
    validateDescriptor(descriptor,version,commit,target,digest(file));
    for(const [asset,hash] of [[name,descriptor.sha256],[descriptor.sources.archive,descriptor.sources.sha256]]){
      allowed.add(asset);allowed.add(asset+'.sha256');
      verifyArchive(directory,asset,hash);
    }
    allowed.add(name+'.json');return descriptor;
  });
  const extras=readdirSync(directory).filter(name=>!allowed.has(name));if(extras.length)throw new Error(`Unexpected/stale release files: ${extras.join(', ')}`);
  return {version,tag:`v${version}`,commit,kind,assets};
}
export function zip(parent,stem,archive){
  if(process.platform==='darwin')run('ditto',['-c','-k','--sequesterRsrc','--keepParent',join(parent,stem),archive]);
  else if(process.platform==='linux')run('zip',['-q','-r',archive,stem],{cwd:parent});
  else run('powershell.exe',['-NoProfile','-NonInteractive','-Command',"$ErrorActionPreference='Stop'; Compress-Archive -LiteralPath $env:MULTICAD_STAGE -DestinationPath $env:MULTICAD_ZIP -CompressionLevel Optimal"],{env:{...process.env,MULTICAD_STAGE:join(parent,stem),MULTICAD_ZIP:archive}});
}
function windowsZip(archive,operation,entry=''){
  const script=`$ErrorActionPreference='Stop'; Add-Type -AssemblyName System.IO.Compression.FileSystem; $zip=[IO.Compression.ZipFile]::OpenRead($env:MULTICAD_ZIP); try { ${operation} } finally { $zip.Dispose() }`;
  return run('powershell.exe',['-NoProfile','-NonInteractive','-Command',script],{
    env:{...process.env,MULTICAD_ZIP:archive,MULTICAD_ENTRY:entry},maxBuffer:32*1024*1024,
  });
}
function readZipEntry(archive,entry){
  if(process.platform!=='win32')return run('unzip',['-p',archive,entry]);
  return windowsZip(archive,"$entries=@($zip.Entries | Where-Object { $_.FullName.Replace([char]92,[char]47) -ceq $env:MULTICAD_ENTRY }); if($entries.Count -ne 1) { throw 'Missing or duplicated archive identity entry' }; $reader=[IO.StreamReader]::new($entries[0].Open()); try { $reader.ReadToEnd() } finally { $reader.Dispose() }",entry);
}
function verifyCandidateEntries(archive,runtime){
  const entries=(process.platform==='win32'
    ?windowsZip(archive,'$zip.Entries | ForEach-Object FullName')
    :run('unzip',['-Z1',archive],{maxBuffer:32*1024*1024})).split(/\r?\n/);
  for(const entry of entries){
    const parts=entry.split(/[/\\]/),name=parts.at(-1);
    if(parts.some(part=>part.toLowerCase()==='native-deps')
      ||/^(?:occt|opencascade).*\.(?:zip|tgz|tar(?:\.gz)?)$/i.test(name)
      ||/^(?:lib)?(?:tk[a-z0-9_]+|occt|opencascade).*\.(?:a|lib|dll|dylib|so(?:\.\d+)*)$/i.test(name)
      ||(runtime&&(name==='native-dependencies.json'||/^occt-/i.test(name)))){
      throw new Error(`Rust candidate archive contains an OpenCascade asset: ${entry}`);
    }
  }
}
function rustCandidateIdentity(version,commit,target){
  requireVersion(version);requireCommit(commit);
  if(!targets.includes(target))throw new Error(`Unsupported Rust candidate target: ${target}`);
  return {version,commit,target,kind:rustCandidateKind,geometryBackend:'rust'};
}
function requireCandidateIdentity(actual,expected){
  if(!actual||Object.entries(expected).some(([key,value])=>actual[key]!==value)||Object.hasOwn(actual,'nativeSourceHash')||Object.hasOwn(actual,'tag'))throw new Error('Mismatched Rust candidate identity');
}
export function validateRustCandidateDiagnostics(diagnostics,version,commit,target){
  rustCandidateIdentity(version,commit,target);
  const [os,arch]=target.split('-'),rustArch={x64:'x86_64',arm64:'aarch64'}[arch];
  const [title,...lines]=diagnostics.split(/\r?\n/);
  const matches=(prefix,expected)=>{const values=lines.filter(line=>line.startsWith(prefix));return values.length===1&&values[0]===prefix+expected;};
  if(title!==`ToskCAD ${version}`||!matches('Target: ',`${os}/${rustArch}`)||!matches('Source commit: ',commit)||!matches('CAD kernel: ','MultiCAD Rust')||lines.some(line=>line.startsWith('OCCT source:')))throw new Error('Executable diagnostics do not identify the expected Rust-only candidate');
}
function rejectOCCTLibraries(libraries){
  if(!Array.isArray(libraries)||libraries.some(library=>typeof library!=='string'||/(?:^|[/\\\s])(?:lib)?(?:tk[a-z0-9]|occt|opencascade)/i.test(library)))throw new Error('Rust candidate contains an invalid runtime library or OpenCascade dependency');
}
export function verifyRustCandidate(directory,version,commit,target){
  const expected=rustCandidateIdentity(version,commit,target);
  const name=rustCandidateArchiveName(version,target),source=rustCandidateSourceName(version,target);
  const descriptor=JSON.parse(readFileSync(join(directory,name+'.json'),'utf8'));
  requireCandidateIdentity(descriptor,expected);
  const sha=value=>typeof value==='string'&&/^[a-f0-9]{64}$/.test(value);
  if(descriptor.archive!==name||descriptor.sources?.archive!==source||!sha(descriptor.sha256)||!sha(descriptor.sources.sha256))throw new Error('Mismatched Rust candidate archive descriptor');
  rejectOCCTLibraries(descriptor.runtimeLibraries);
  for(const [asset,hash] of [[name,descriptor.sha256],[source,descriptor.sources.sha256]]){
    verifyArchive(directory,asset,hash);
    const path=join(directory,asset);
    verifyCandidateEntries(path,asset===name);
    const identity=JSON.parse(readZipEntry(path,asset.slice(0,-4)+'/BUILD-IDENTITY.json'));
    requireCandidateIdentity(identity,expected);
  }
  const diagnostics=readZipEntry(join(directory,name),name.slice(0,-4)+'/BUILD-IDENTITY.txt');
  validateRustCandidateDiagnostics(diagnostics,version,commit,target);
  return descriptor;
}
export async function buildPackage(version,{rustOnly=false}={}){
  requireVersion(version);
  if(run('git',['status','--porcelain','--untracked-files=no']))throw new Error('Commit tracked changes before packaging a source-identified release');
  const commit=requireCommit(run('git',['rev-parse','HEAD']));
  const target=hostTarget();
  if(!targets.includes(target))throw new Error(`Unsupported packaging host: ${target}`);
  const env={...process.env,MULTICAD_RELEASE_VERSION:version,MULTICAD_BUILD_COMMIT:commit};
  if(rustOnly)env.CARGO_TARGET_DIR=join(root,'target/cargo-rust-candidate');
  else run(process.execPath,[join(root,'tools/foundation-deps.mjs')],{stdio:'inherit'});
  run('cargo',['fetch','--locked'],{stdio:'inherit'});
  if(rustOnly){
    run('cargo',['test','-p','multicad-app','--locked','--no-default-features','--test','cli','--test','command'],{env,stdio:'inherit'});
    run('cargo',['test','-p','multicad-app','--locked','--no-default-features','--test','mcp','rust_stdio'],{env,stdio:'inherit'});
    run('cargo',['build',...rustCandidateBuildArgs],{env,stdio:'inherit'});
  }else{
    run('cargo',['test','--workspace','--locked'],{env,stdio:'inherit'});
    // Keep the compiler-aware native cache and package dependency caches.
    run('cargo',['clean','--release','-p','multicad-app','-p','multicad-ui','-p','multicad-core','-p','multicad-kernel','-p','multicad-solver'],{stdio:'inherit'});
    run('cargo',['build','--workspace','--release','--locked'],{env,stdio:'inherit'});
  }
  const metadata=JSON.parse(run('cargo',['metadata','--no-deps','--format-version','1','--locked'],{env}));
  const binary=join(metadata.target_directory,'release',process.platform==='win32'?'multicad.exe':'multicad');
  if(run(binary,['--version'])!==`ToskCAD ${version}`)throw new Error('Compiled version does not match package');
  let diagnostics=run(binary,['--diagnostics']);
  let identity;
  if(rustOnly){
    identity=rustCandidateIdentity(version,commit,target);
    validateRustCandidateDiagnostics(diagnostics,version,commit,target);
  }else{
    const nativeSourceHashValue=nativeSourceHash(join(root,'artifacts/native-deps/OCCT-7_9_3'));
    if(!diagnostics.includes(`OCCT source: ${nativeSourceHashValue}`))throw new Error('Binary native source identity does not match the source package');
    const [os,arch]=target.split('-'),rustArch={x64:'x86_64',arm64:'aarch64'}[arch];
    if(!diagnostics.includes(`Source commit: ${commit}`)||!diagnostics.includes(`Target: ${os}/${rustArch}`))throw new Error('Compiled identity/target does not match source/host');
    identity={version,tag:`v${version}`,commit,target,kind,nativeSourceHash:nativeSourceHashValue};
  }
  const output=rustOnly?rustCandidateDirectory:outputDirectory;
  mkdirSync(output,{recursive:true});mkdirSync(join(root,'artifacts/staging'),{recursive:true});
  const scratch=mkdtempSync(join(root,'artifacts/staging/native-'));
  try{
    const name=(rustOnly?rustCandidateArchiveName:archiveName)(version,target),stem=name.slice(0,-4),stage=join(scratch,stem);
    const sources=(rustOnly?rustCandidateSourceName:sourceName)(version,target),sourceStem=sources.slice(0,-4),source=join(scratch,sourceStem);
    const sourceMetadata=createSourceTree(source,commit,version,{rustOnly});
    const layout=assembleRuntime(stage,binary,version,{rustOnly});collectNotices(layout.resources,source,sourceMetadata);
    const runtimeLibraries=auditRuntime(layout.executable);
    if(rustOnly){
      rejectOCCTLibraries(runtimeLibraries);
      diagnostics=run(layout.executable,['--diagnostics'],{cwd:scratch});
      validateRustCandidateDiagnostics(diagnostics,version,commit,target);
      for(const folder of [stage,source])writeFileSync(join(folder,'BUILD-IDENTITY.json'),JSON.stringify(identity,null,2)+'\n');
    }
    run(layout.executable,['--self-test',join(scratch,'smoke')],{cwd:scratch,stdio:'inherit'});
    writeFileSync(join(stage,'BUILD-IDENTITY.txt'),diagnostics+'\n');
    zip(scratch,stem,join(scratch,name));zip(scratch,sourceStem,join(scratch,sources));
    const hash=digest(join(scratch,name)),sourceHash=digest(join(scratch,sources));
    const descriptor={...identity,archive:name,sha256:hash,sources:{archive:sources,sha256:sourceHash},runtimeLibraries,signing:process.platform==='darwin'?'ad-hoc, not notarized':'unsigned'};
    writeFileSync(join(scratch,name+'.json'),JSON.stringify(descriptor,null,2)+'\n');
    for(const [asset,sha] of [[name,hash],[sources,sourceHash]])writeFileSync(join(scratch,asset+'.sha256'),`${sha}  ${asset}\n`);
    if(rustOnly)verifyRustCandidate(scratch,version,commit,target);
    for(const file of [name,name+'.json',name+'.sha256',sources,sources+'.sha256'])renameSync(join(scratch,file),join(output,file));
    if(run('git',['status','--porcelain','--untracked-files=no']))throw new Error('Packaging modified tracked files');
    console.log(JSON.stringify(descriptor,null,2));return descriptor;
  }finally{rmSync(scratch,{recursive:true,force:true});}
}
if(process.argv[1]&&realpathSync(process.argv[1])===realpathSync(fileURLToPath(import.meta.url))){
  const [action,version,commit,...extra]=process.argv.slice(2);
  try{
    if(extra.length)throw new Error('Unexpected arguments');
    if(action==='build'&&version&&!commit)await buildPackage(version);
    else if(action==='build-rust-candidate'&&version&&!commit)await buildPackage(version,{rustOnly:true});
    else if(action==='verify'&&version&&commit){const manifest=verifyPackageSet(outputDirectory,version,commit);writeFileSync(join(outputDirectory,'release-manifest.json'),JSON.stringify(manifest,null,2)+'\n');console.log(`Verified ${manifest.assets.length} native archives and matching sources for v${version} at ${commit}`);}
    else if(action==='verify-rust-candidate'&&version&&commit){const descriptor=verifyRustCandidate(rustCandidateDirectory,version,commit,hostTarget());console.log(`Verified ${descriptor.archive} and matching sources at ${commit}`);}
    else throw new Error('Usage: package-release.mjs build VERSION | verify VERSION COMMIT | build-rust-candidate VERSION | verify-rust-candidate VERSION COMMIT');
  }catch(error){console.error(error.message);process.exitCode=1;}
}
