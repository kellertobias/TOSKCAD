// Developer/CI orchestration only. The shipped application contains no Node.js.
import { execFileSync, spawn } from 'node:child_process';
import { existsSync, mkdirSync, mkdtempSync, readFileSync, realpathSync, renameSync, writeFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { assembleRuntime } from './native-package.mjs';

export const root=resolve(dirname(fileURLToPath(import.meta.url)),'..');
const run=(command,args=[],options={})=>execFileSync(command,args,{cwd:root,stdio:'inherit',...options});
const capture=(command,args=[])=>execFileSync(command,args,{cwd:root,encoding:'utf8'}).trim();

export function bootstrap(){
  const [major,minor]=process.versions.node.split('.').map(Number);
  if(major<22||(major===22&&minor<14))throw new Error('Install Node.js 22.14 or newer');
  for(const [command,args,hint] of [
    ['rustup',['--version'],'Install Rust from https://rustup.rs/'],
    ['cmake',['--version'],'Install CMake 3.16+ (macOS: brew install cmake)'],
    ['tar',['--version'],'Install tar (included with supported macOS/Windows/Linux)'],
  ]){try{capture(command,args);}catch{throw new Error(hint);}}
  if(process.platform==='darwin'){
    try{capture('xcrun',['--find','clang++']);}catch{throw new Error('Install Apple Command Line Tools: xcode-select --install');}
  }
  // rustup selects/installs the repository pin without changing global defaults.
  run('rustup',['show','active-toolchain']);
  run(process.execPath,[join(root,'tools/foundation-deps.mjs')]);
  run('cargo',['fetch','--locked']);
}
export function version(){
  return readFileSync(join(root,'Cargo.toml'),'utf8').match(/\[workspace\.package\][\s\S]*?^version = "([^"]+)"/m)?.[1]??'0.1.0-dev';
}
export function developmentExecutable(environment=process.env){
  const metadata=JSON.parse(execFileSync('cargo',['metadata','--no-deps','--format-version','1','--locked','--offline'],{cwd:root,env:environment,encoding:'utf8'}));
  if(typeof metadata.target_directory!=='string'||!metadata.target_directory)throw new Error('Cargo did not report its build output directory');
  return join(metadata.target_directory,'debug',process.platform==='win32'?'multicad.exe':'multicad');
}
function releaseDependencies(){
  if(!existsSync(join(root,'node_modules/semver/package.json')))run(process.platform==='win32'?'npm.cmd':'npm',['ci','--ignore-scripts','--no-audit','--no-fund']);
}
function executable(bundle){return process.platform==='darwin'?join(bundle,'MultiCAD.app/Contents/MacOS/multicad'):join(bundle,process.platform==='win32'?'multicad.exe':'multicad');}

async function main(){
  const args=process.argv.slice(2);
  if(args.includes('--help')){console.log('Usage: ./build [open | check | archive [install]]\nWindows: ./build.ps1 [same arguments]\nArchives are built for the current host; other platforms use GitHub runners.');return;}
  if(args.length>2||!['','open','check','archive'].includes(args[0]??'')||(args.length===2&&(args[0]!=='archive'||args[1]!=='install')))throw new Error('Unsupported action; see ./build --help');
  bootstrap();
  if(args[0]==='check'){
    run('cargo',['fmt','--all','--','--check']);run('cargo',['clippy','--workspace','--all-targets','--locked','--','-D','warnings']);run('cargo',['test','--workspace','--locked']);releaseDependencies();run(process.platform==='win32'?'npm.cmd':'npm',['test']);return;
  }
  if(args[0]==='archive'){
    releaseDependencies();
    const {buildPackage}=await import('./package-release.mjs');
    const descriptor=await buildPackage(process.env.MULTICAD_RELEASE_VERSION??version());
    console.log(`Archive ready: ${descriptor.archive}`);
    if(args[1]==='install'){
      if(process.platform!=='darwin')throw new Error('Automatic install is currently supported on macOS only; extract the portable host archive to install on Windows/Linux.');
      // Installation is only reached by an explicit invocation of archive install.
      const bundle=join(root,'artifacts/install');mkdirSync(bundle,{recursive:true});
      run('ditto',['-x','-k',join(root,'artifacts/release',descriptor.archive),bundle]);
      const source=join(bundle,descriptor.archive.slice(0,-4),'MultiCAD.app');
      const destination='/Applications/MultiCAD.app';
      // Never kill an independently launched/installed session. Ask for its
      // normal close first; future document hosts must supply a graceful API.
      let running='';try{running=capture('pgrep',['-f','^/Applications/MultiCAD\\.app/Contents/MacOS/multicad$']);}catch{}
      if(running)throw new Error('Close the installed MultiCAD through its normal Quit action, then rerun archive install.');
      const incoming=`/Applications/.MultiCAD-incoming-${process.pid}.app`,backup=`/Applications/.MultiCAD-previous-${Date.now()}.app`;
      run('ditto',[source,incoming]);
      let replaced=false;
      try{if(existsSync(destination)){renameSync(destination,backup);replaced=true;}renameSync(incoming,destination);}
      catch(error){if(replaced&&!existsSync(destination))renameSync(backup,destination);throw error;}
      run('open',[destination]);
    }return;
  }
  run('cargo',['build','--workspace','--locked']);
  const binary=developmentExecutable();
  const parent=join(root,'artifacts/development');mkdirSync(parent,{recursive:true});
  const bundle=mkdtempSync(join(parent,'build-'));assembleRuntime(bundle,binary,version());
  writeFileSync(join(parent,'latest.json'),JSON.stringify({bundle,executable:executable(bundle)}));
  console.log(`Native application: ${executable(bundle)}`);
  if(args[0]==='open'){
    // Editable windows own their close prompts. Opening a new window preserves them.
    const path=executable(bundle);
    const child=spawn(path,[],{cwd:bundle,stdio:'ignore',detached:true});
    child.on('error',error=>{console.error(`Could not start MultiCAD: ${error.message}`);process.exitCode=1;});
    await new Promise((ok,fail)=>{child.once('spawn',ok);child.once('error',fail);});
    mkdirSync(join(root,'artifacts/runtime'),{recursive:true});
    writeFileSync(join(root,'artifacts/runtime/instance.json'),JSON.stringify({pid:child.pid,path}));child.unref();
  }
}
if(process.argv[1]&&realpathSync(process.argv[1])===realpathSync(fileURLToPath(import.meta.url)))main().catch(error=>{console.error(error.message);process.exitCode=1;});
