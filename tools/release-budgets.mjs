// Developer-only release-budget driver (Node.js is not a runtime dependency).
// Builds the `release_budgets` example, generates the four acceptance
// scenario projects once, then measures each scenario in fresh processes and
// records wall-clock phases plus peak resident memory from /usr/bin/time.
//
//   node tools/release-budgets.mjs [--profile release|dev] [--repeat N] [--label NAME]
//
// Output: artifacts/benchmarks/<label>/report.json (git-ignored). This is a
// headless measurement; GUI frame times and idle CPU are not covered.
import {execFileSync,spawnSync} from 'node:child_process';
import {existsSync,mkdirSync,readFileSync,rmSync,writeFileSync} from 'node:fs';
import {dirname,join,resolve} from 'node:path';
import {fileURLToPath} from 'node:url';
import os from 'node:os';

const root=resolve(dirname(fileURLToPath(import.meta.url)),'..');
const scenarios=['makerspace','cabinetry_table','tool_case','control_desk'];
const options={profile:'release',repeat:3,label:new Date().toISOString().slice(0,10)};
const argv=process.argv.slice(2);
for(let i=0;i<argv.length;i+=2){
  const [flag,value]=[argv[i],argv[i+1]];
  if(flag==='--profile'&&['release','dev'].includes(value))options.profile=value;
  else if(flag==='--repeat'&&Number(value)>=1)options.repeat=Number(value);
  else if(flag==='--label'&&/^[\w.-]+$/.test(value??''))options.label=value;
  else{console.error('Usage: release-budgets.mjs [--profile release|dev] [--repeat N] [--label NAME]');process.exit(2);}
}
const capture=(command,args)=>{try{return execFileSync(command,args,{cwd:root,encoding:'utf8',stdio:['ignore','pipe','ignore']}).trim();}catch{return null;}};
function machine(){
  const info={platform:process.platform,arch:process.arch,os_release:os.release(),cpus:os.cpus().length,memory_bytes:os.totalmem(),cpu_model:os.cpus()[0]?.model??null};
  if(process.platform==='darwin'){
    info.cpu_model=capture('sysctl',['-n','machdep.cpu.brand_string'])??info.cpu_model;
    info.hardware_model=capture('sysctl',['-n','hw.model']);
    info.physical_cores=Number(capture('sysctl',['-n','hw.physicalcpu']));
    info.performance_cores=Number(capture('sysctl',['-n','hw.perflevel0.physicalcpu']))||null;
    info.efficiency_cores=Number(capture('sysctl',['-n','hw.perflevel1.physicalcpu']))||null;
    info.memory_bytes=Number(capture('sysctl',['-n','hw.memsize']));
    info.macos=capture('sw_vers',['-productVersion']);
    info.power_source=capture('pmset',['-g','batt'])?.split('\n')[0]??null;
  }
  return info;
}
// Peak resident set size in bytes from the platform's time(1).
function timed(args){
  const timeArgs=process.platform==='darwin'?['-l']:process.platform==='linux'?['-v']:null;
  const result=timeArgs&&existsSync('/usr/bin/time')
    ?spawnSync('/usr/bin/time',[...timeArgs,...args],{cwd:root,encoding:'utf8',maxBuffer:256*1024*1024})
    :spawnSync(args[0],args.slice(1),{cwd:root,encoding:'utf8',maxBuffer:256*1024*1024});
  if(result.status!==0){
    process.stderr.write(result.stderr.split('\n').slice(-40).join('\n'));
    throw new Error(`${args.slice(1).join(' ')} failed with status ${result.status}`);
  }
  let rss=null;
  const mac=/^\s*(\d+)\s+maximum resident set size/m.exec(result.stderr);
  const linux=/Maximum resident set size \(kbytes\):\s*(\d+)/.exec(result.stderr);
  if(mac)rss=Number(mac[1]);else if(linux)rss=Number(linux[1])*1024;
  return rss;
}

const cargoProfile=options.profile==='release'?['--release']:[];
execFileSync('cargo',['build','-p','multicad-app','--example','release_budgets','--locked','--offline',...cargoProfile],{cwd:root,stdio:'inherit'});
const metadata=JSON.parse(execFileSync('cargo',['metadata','--no-deps','--format-version','1','--locked','--offline'],{cwd:root,encoding:'utf8'}));
const binary=join(metadata.target_directory,options.profile==='release'?'release':'debug','examples',process.platform==='win32'?'release_budgets.exe':'release_budgets');
const output=join(root,'artifacts/benchmarks',options.label);
rmSync(output,{recursive:true,force:true});mkdirSync(output,{recursive:true});

const report={
  generated_at:new Date().toISOString(),
  profile:options.profile,
  repeat:options.repeat,
  commit:capture('git',['rev-parse','HEAD']),
  dirty:Boolean(capture('git',['status','--porcelain','--untracked-files=no'])),
  rustc:capture('rustc',['--version']),
  machine:machine(),
  generate:null,
  scenarios:{},
};
console.error(`Generating scenario projects in ${output}`);
const generateRss=timed([binary,'generate',output]);
report.generate={peak_rss_bytes:generateRss,...JSON.parse(readFileSync(join(output,'generate.json'),'utf8'))};
for(const scenario of scenarios){
  report.scenarios[scenario]=[];
  for(let run=0;run<options.repeat;run++){
    console.error(`Measuring ${scenario} (${run+1}/${options.repeat})`);
    const rss=timed([binary,'measure',scenario,output]);
    report.scenarios[scenario].push({peak_rss_bytes:rss,...JSON.parse(readFileSync(join(output,scenario,'measure.json'),'utf8'))});
  }
}
writeFileSync(join(output,'report.json'),JSON.stringify(report,null,2)+'\n');
console.log(join(output,'report.json'));
