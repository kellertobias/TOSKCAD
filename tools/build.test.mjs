import assert from 'node:assert/strict';
import { mkdtempSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';
import { developmentExecutable, root } from './build.mjs';

test('development packaging follows the actual Cargo target override',()=>{
  const directory=mkdtempSync(join(tmpdir(),'multicad-cargo-output-'));
  try{
    const expected=join(directory,'debug',process.platform==='win32'?'multicad.exe':'multicad');
    mkdirSync(join(directory,'debug'));
    writeFileSync(expected,'fresh selected output');
    const actual=developmentExecutable({...process.env,CARGO_TARGET_DIR:directory});
    assert.equal(actual,expected);
    assert.equal(readFileSync(actual,'utf8'),'fresh selected output');
  }finally{rmSync(directory,{recursive:true,force:true});}
});

test('normal Cargo and developer packaging agree on the configured output',()=>{
  const environment={...process.env};delete environment.CARGO_TARGET_DIR;
  assert.equal(developmentExecutable(environment),join(root,'target/cargo/debug',process.platform==='win32'?'multicad.exe':'multicad'));
});
