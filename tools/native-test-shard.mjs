import {execFileSync} from 'node:child_process';
import {dirname,resolve} from 'node:path';
import {fileURLToPath,pathToFileURL} from 'node:url';

export const exampleGeneratorFilter='generator_rebuilds_the_shipped_project';
export function workspaceTestArguments(shard='full'){
  const args=['test','--workspace','--profile','ci','--locked'];
  if(shard==='generators')args.push(exampleGeneratorFilter);
  else if(shard==='remaining')args.push('--','--skip',exampleGeneratorFilter);
  else if(shard!=='full')throw new Error(`Unknown native test shard: ${shard}`);
  return args;
}
export function packageTestArguments(shard='full',platform=process.platform){
  if(shard!=='full'&&!(platform==='win32'&&shard==='remaining'))throw new Error('Only the paired Windows package job may select the remaining test shard');
  return workspaceTestArguments(shard);
}

if(process.argv[1]&&import.meta.url===pathToFileURL(resolve(process.argv[1])).href){
  try{
    execFileSync('cargo',workspaceTestArguments(process.argv[2]??'full'),{
      cwd:resolve(dirname(fileURLToPath(import.meta.url)),'..'),stdio:'inherit',
    });
  }catch(error){
    console.error(error.message);
    process.exitCode=error.status??1;
  }
}
