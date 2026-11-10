import assert from 'node:assert/strict';
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';
import { archiveName, sourceName, digest, rustCandidateArchiveName, targets, validateDescriptor, validateRustCandidateDiagnostics, verifyPackageSet, verifyRustCandidate, zip } from './package-release.mjs';

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

const candidateTarget='macos-arm64';
const candidateIdentity={version,commit,target:candidateTarget,kind:'rust-only-development-host',geometryBackend:'rust'};
const candidateDiagnostics=`ToskCAD ${version}\nTarget: macos/aarch64\nSource commit: ${commit}\nCAD kernel: MultiCAD Rust\n`;

test('candidate diagnostics demand exact version, commit, target and Rust-only kernel',()=>{
  const verify=text=>validateRustCandidateDiagnostics(text,version,commit,candidateTarget);
  verify(candidateDiagnostics);
  verify(candidateDiagnostics.replaceAll('\n','\r\n'));
  for(const text of [
    candidateDiagnostics.replace(version,'1.2.30'),
    candidateDiagnostics.replace(commit,'b'.repeat(40)),
    candidateDiagnostics.replace('macos/aarch64','macos/x86_64'),
    candidateDiagnostics.replace('MultiCAD Rust','OCCT 7.9.3 (statically linked)'),
    candidateDiagnostics+'OCCT source: stale\n',
    candidateDiagnostics+'CAD kernel: OCCT 7.9.3 (statically linked)\n',
  ])assert.throws(()=>verify(text),/diagnostics/);
});

test('candidate archive verification binds both ZIPs, binary diagnostics and the local-only descriptor',()=>{
  const dir=mkdtempSync(join(tmpdir(),'multicad-rust-candidate-'));
  try{
    const name=rustCandidateArchiveName(version,candidateTarget);
    const source=name.replace(/\.zip$/,'-sources.zip');
    const runtimeIdentity=join(dir,name.slice(0,-4),'BUILD-IDENTITY.json');
    const runtimeDiagnostics=join(dir,name.slice(0,-4),'BUILD-IDENTITY.txt');
    const sourceIdentity=join(dir,source.slice(0,-4),'BUILD-IDENTITY.json');
    for(const stem of [name,source].map(file=>file.slice(0,-4)))mkdirSync(join(dir,stem));
    writeFileSync(runtimeIdentity,JSON.stringify(candidateIdentity));
    writeFileSync(runtimeDiagnostics,candidateDiagnostics);
    writeFileSync(sourceIdentity,JSON.stringify(candidateIdentity));
    const archive=asset=>{
      const file=join(dir,asset);rmSync(file,{force:true});zip(dir,asset.slice(0,-4),file);
      const hash=digest(file);writeFileSync(file+'.sha256',`${hash}  ${asset}\n`);return hash;
    };
    const descriptor={...candidateIdentity,archive:name,sha256:archive(name),sources:{archive:source,sha256:archive(source)},runtimeLibraries:['/usr/lib/libSystem.B.dylib']};
    const writeDescriptor=()=>writeFileSync(join(dir,name+'.json'),JSON.stringify(descriptor));
    const verify=()=>verifyRustCandidate(dir,version,commit,candidateTarget);
    writeDescriptor();
    assert.deepEqual(verify(),descriptor);
    assert.throws(()=>validateDescriptor(descriptor,version,commit,candidateTarget,descriptor.sha256),/Mismatched release asset/);

    descriptor.runtimeLibraries.push('/usr/lib/libTKernel.dylib');
    writeDescriptor();assert.throws(verify,/OpenCascade/);
    descriptor.runtimeLibraries.pop();
    descriptor.nativeSourceHash='c'.repeat(64);
    writeDescriptor();assert.throws(verify,/candidate identity/);
    delete descriptor.nativeSourceHash;

    writeFileSync(sourceIdentity,JSON.stringify({...candidateIdentity,commit:'b'.repeat(40)}));
    descriptor.sources.sha256=archive(source);
    writeDescriptor();assert.throws(verify,/candidate identity/);
    writeFileSync(sourceIdentity,JSON.stringify(candidateIdentity));
    descriptor.sources.sha256=archive(source);

    const native=join(dir,source.slice(0,-4),'artifacts/native-deps');
    mkdirSync(native,{recursive:true});
    writeFileSync(join(native,'occt-7.9.3.tar.gz'),'unexpected native source');
    descriptor.sources.sha256=archive(source);
    writeDescriptor();assert.throws(verify,/OpenCascade asset/);
    rmSync(native,{recursive:true});
    descriptor.sources.sha256=archive(source);

    writeFileSync(runtimeDiagnostics,candidateDiagnostics.replace('MultiCAD Rust','OCCT'));
    descriptor.sha256=archive(name);
    writeDescriptor();assert.throws(verify,/diagnostics/);
    writeFileSync(runtimeDiagnostics,candidateDiagnostics);
    descriptor.sha256=archive(name);
    writeDescriptor();
    writeFileSync(join(dir,source+'.sha256'),'stale checksum');
    assert.throws(verify,/Checksum mismatch/);
  }finally{rmSync(dir,{recursive:true,force:true});}
});
