import assert from 'node:assert/strict';
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';
import { archiveName, sourceName, digest, targets, validateDescriptor, verifyPackageSet } from './package-release.mjs';

const version='1.2.3',commit='a'.repeat(40);
function fixture(){
  const dir=mkdtempSync(join(tmpdir(),'multicad-package-contract-'));
  for(const target of targets){
    const name=archiveName(version,target),file=join(dir,name);
    // Contract fixture, not a substitute for the real archiver smoke check.
    writeFileSync(file,Buffer.concat([Buffer.from([0x50,0x4b,0x03,0x04]),Buffer.from(target)]));
    const sha256=digest(file);
    const sources=sourceName(version,target);const sourceFile=join(dir,sources);
    writeFileSync(sourceFile,Buffer.concat([Buffer.from([0x50,0x4b,0x03,0x04]),Buffer.from('source contract fixture')]));
    const sourceHash=digest(sourceFile);
    writeFileSync(sourceFile+'.sha256',`${sourceHash}  ${sources}\n`);
    writeFileSync(file+'.json',JSON.stringify({version,tag:`v${version}`,commit,target,archive:name,sha256,kind:'native-development-host',nativeSourceHash:'c'.repeat(64),sources:{archive:sources,sha256:sourceHash}}));
    writeFileSync(file+'.sha256',`${sha256}  ${name}\n`);
  }
  return dir;
}

test('aggregate verification requires all three exact version/commit/target identities and is repeatable',()=>{
  const dir=fixture();try{
    const manifest=verifyPackageSet(dir,version,commit);assert.equal(manifest.assets.length,3);
    writeFileSync(join(dir,'release-manifest.json'),JSON.stringify(manifest));
    assert.deepEqual(verifyPackageSet(dir,version,commit),manifest);
  }finally{rmSync(dir,{recursive:true,force:true});}
});
for(const scenario of ['missing target','changed bytes','wrong commit','wrong tag','stale asset','missing sources','changed sources'])test(`package validation rejects ${scenario}`,()=>{
  const dir=fixture();try{
    const file=join(dir,archiveName(version,targets[0]));
    if(scenario==='missing target')rmSync(file);
    if(scenario==='changed bytes')writeFileSync(file,'damaged');
    if(scenario==='wrong commit'){const d=JSON.parse(readFileSync(file+'.json'));d.commit='b'.repeat(40);writeFileSync(file+'.json',JSON.stringify(d));}
    if(scenario==='wrong tag'){const d=JSON.parse(readFileSync(file+'.json'));d.tag='v9.9.9';writeFileSync(file+'.json',JSON.stringify(d));}
    if(scenario==='stale asset')writeFileSync(join(dir,'old-release.zip'),'stale');
    if(scenario==='missing sources')rmSync(join(dir,sourceName(version,targets[0])));
    if(scenario==='changed sources')writeFileSync(join(dir,sourceName(version,targets[0])),'changed');
    assert.throws(()=>verifyPackageSet(dir,version,commit));
  }finally{rmSync(dir,{recursive:true,force:true});}
});

test('descriptor validation rejects an unknown target and invalid SemVer',()=>{
  assert.throws(()=>validateDescriptor({},version,commit,'unknown','hash'));
  assert.throws(()=>archiveName('../bad','linux-x64'));
});
