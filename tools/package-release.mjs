import {execFileSync} from 'node:child_process';
import {createHash} from 'node:crypto';
import {existsSync,mkdirSync,mkdtempSync,readFileSync,readdirSync,realpathSync,renameSync,rmSync,writeFileSync} from 'node:fs';
import {dirname,join,resolve} from 'node:path';
import {fileURLToPath,pathToFileURL} from 'node:url';
import {requireCommit,requireVersion} from './release-policy.mjs';
import {assembleRuntime,auditRuntime,collectNotices,createSourceTree,nativeSourceHash} from './native-package.mjs';
export {assembleRuntime} from './native-package.mjs';

const root=resolve(dirname(fileURLToPath(import.meta.url)),'..');
export const targets=Object.freeze(['linux-x64','macos-arm64','windows-x64']);
export const outputDirectory=join(root,'artifacts/release');
const kind='native-development-host';
const run=(command,args,options={})=>execFileSync(command,args,{cwd:root,encoding:'utf8',stdio:['ignore','pipe','pipe'],...options})?.trim()??'';
export const digest=path=>createHash('sha256').update(readFileSync(path)).digest('hex');
export const archiveName=(version,target)=>`multicad-${requireVersion(version)}-${target}.zip`;
export const sourceName=(version,target)=>`multicad-${requireVersion(version)}-${target}-sources.zip`;
export function validateDescriptor(value,version,commit,target,archiveHash){
  requireVersion(version);requireCommit(commit);
  if(!targets.includes(target)||value.version!==version||value.tag!==`v${version}`||value.commit!==commit||value.target!==target||value.sha256!==archiveHash||value.archive!==archiveName(version,target)||value.kind!==kind||!/^[a-f0-9]{64}$/.test(value.nativeSourceHash??'')||value.sources?.archive!==sourceName(version,target)||!/^[a-f0-9]{64}$/.test(value.sources?.sha256??''))throw new Error(`Mismatched release asset identity for ${target}`);
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
      if(digest(join(directory,asset))!==hash||readFileSync(join(directory,asset+'.sha256'),'utf8')!==`${hash}  ${asset}\n`)throw new Error(`Checksum mismatch for ${asset}`);
      if(readFileSync(join(directory,asset)).subarray(0,4).toString('hex')!=='504b0304')throw new Error(`Not a ZIP archive: ${asset}`);
    }
    allowed.add(name+'.json');return descriptor;
  });
  const extras=readdirSync(directory).filter(name=>!allowed.has(name));if(extras.length)throw new Error(`Unexpected/stale release files: ${extras.join(', ')}`);
  return {version,tag:`v${version}`,commit,kind,assets};
}
function zip(parent,stem,archive){
  if(process.platform==='darwin')run('ditto',['-c','-k','--sequesterRsrc','--keepParent',join(parent,stem),archive]);
  else if(process.platform==='linux')run('zip',['-q','-r',archive,stem],{cwd:parent});
  else run('powershell.exe',['-NoProfile','-NonInteractive','-Command',"$ErrorActionPreference='Stop'; Compress-Archive -LiteralPath $env:MULTICAD_STAGE -DestinationPath $env:MULTICAD_ZIP -CompressionLevel Optimal"],{env:{...process.env,MULTICAD_STAGE:join(parent,stem),MULTICAD_ZIP:archive}});
}
export async function buildPackage(version){
  requireVersion(version);
  if(run('git',['status','--porcelain','--untracked-files=no']))throw new Error('Commit tracked changes before packaging a source-identified release');
  const commit=requireCommit(run('git',['rev-parse','HEAD']));
  const target=`${{darwin:'macos',win32:'windows',linux:'linux'}[process.platform]}-${process.arch}`;
  if(!targets.includes(target))throw new Error(`Unsupported packaging host: ${target}`);
  const env={...process.env,MULTICAD_RELEASE_VERSION:version,MULTICAD_BUILD_COMMIT:commit};
  run(process.execPath,[join(root,'tools/foundation-deps.mjs')],{stdio:'inherit'});
  run('cargo',['fetch','--locked'],{stdio:'inherit'});
  run('cargo',['test','--workspace','--locked'],{env,stdio:'inherit'});
  // Clean shipped workspace products; immutable target/compiler-aware native
  // cache and package-manager dependency caches remain reusable.
  run('cargo',['clean','--release','-p','multicad-app','-p','multicad-ui','-p','multicad-core','-p','multicad-kernel','-p','multicad-solver'],{stdio:'inherit'});
  run('cargo',['build','--workspace','--release','--locked'],{env,stdio:'inherit'});
  const metadata=JSON.parse(run('cargo',['metadata','--no-deps','--format-version','1','--locked']));
  const binary=join(metadata.target_directory,'release',process.platform==='win32'?'multicad.exe':'multicad');
  if(run(binary,['--version'])!==`ToskCAD ${version}`)throw new Error('Compiled version does not match package');
  const diagnostics=run(binary,['--diagnostics']);
  const nativeIdentity=nativeSourceHash(join(root,'artifacts/native-deps/OCCT-7_9_3'));
  if(!diagnostics.includes(`OCCT source: ${nativeIdentity}`))throw new Error('Binary native source identity does not match the source package');
  const rustOS={darwin:'macos',win32:'windows',linux:'linux'}[process.platform],rustArch={x64:'x86_64',arm64:'aarch64'}[process.arch];
  if(!diagnostics.includes(`Source commit: ${commit}`)||!diagnostics.includes(`Target: ${rustOS}/${rustArch}`))throw new Error('Compiled identity/target does not match source/host');
  mkdirSync(outputDirectory,{recursive:true});mkdirSync(join(root,'artifacts/staging'),{recursive:true});
  const scratch=mkdtempSync(join(root,'artifacts/staging/native-'));
  try{
    const name=archiveName(version,target),stem=name.slice(0,-4),stage=join(scratch,stem);
    const sources=sourceName(version,target),sourceStem=sources.slice(0,-4),source=join(scratch,sourceStem);
    const sourceMetadata=createSourceTree(source,commit,version);
    const layout=assembleRuntime(stage,binary,version);collectNotices(layout.resources,source,sourceMetadata);
    const runtimeLibraries=auditRuntime(layout.executable);
    run(layout.executable,['--self-test',join(scratch,'smoke')],{cwd:scratch,stdio:'inherit'});
    writeFileSync(join(stage,'BUILD-IDENTITY.txt'),diagnostics+'\n');
    zip(scratch,stem,join(scratch,name));zip(scratch,sourceStem,join(scratch,sources));
    const hash=digest(join(scratch,name)),sourceHash=digest(join(scratch,sources));
    const descriptor={version,tag:`v${version}`,commit,target,archive:name,sha256:hash,kind,nativeSourceHash:nativeIdentity,sources:{archive:sources,sha256:sourceHash},runtimeLibraries,signing:process.platform==='darwin'?'ad-hoc, not notarized':'unsigned'};
    writeFileSync(join(scratch,name+'.json'),JSON.stringify(descriptor,null,2)+'\n');
    for(const [asset,sha] of [[name,hash],[sources,sourceHash]])writeFileSync(join(scratch,asset+'.sha256'),`${sha}  ${asset}\n`);
    for(const file of [name,name+'.json',name+'.sha256',sources,sources+'.sha256'])renameSync(join(scratch,file),join(outputDirectory,file));
    if(run('git',['status','--porcelain','--untracked-files=no']))throw new Error('Packaging modified tracked files');
    console.log(JSON.stringify(descriptor,null,2));return descriptor;
  }finally{rmSync(scratch,{recursive:true,force:true});}
}
if(process.argv[1]&&realpathSync(process.argv[1])===realpathSync(fileURLToPath(import.meta.url))){
  const [action,version,commit,...extra]=process.argv.slice(2);
  try{
    if(extra.length)throw new Error('Unexpected arguments');
    if(action==='build'&&version&&!commit)await buildPackage(version);
    else if(action==='verify'&&version&&commit){const manifest=verifyPackageSet(outputDirectory,version,commit);writeFileSync(join(outputDirectory,'release-manifest.json'),JSON.stringify(manifest,null,2)+'\n');console.log(`Verified ${manifest.assets.length} native archives and matching sources for v${version} at ${commit}`);}
    else throw new Error('Usage: package-release.mjs build VERSION | verify VERSION COMMIT');
  }catch(error){console.error(error.message);process.exitCode=1;}
}
