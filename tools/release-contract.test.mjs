import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { createRequire } from 'node:module';
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';
import { githubRepositoryUrl, requirePublicationEnvironment, requireVersion } from './release-policy.mjs';

const require = createRequire(import.meta.url);
const config = require('../release.config.cjs');
const git = (cwd, args) => execFileSync('git', args, {cwd, encoding:'utf8', stdio:['ignore','pipe','pipe']}).trim();
function preview(sourceRoot) {
  // semantic-release hooks stdout; isolate it from node:test's worker protocol.
  const environment={...process.env};delete environment.NODE_TEST_CONTEXT;
  const program=`import { Writable } from 'node:stream';
    import { runRelease } from ${JSON.stringify(new URL('./run-semantic-release.mjs',import.meta.url).href)};
    const sink=new Writable({write(_chunk,_encoding,done){done();}});
    const result=await runRelease({dryRun:true,sourceRoot:process.argv[1],stdout:sink,stderr:sink});
    console.log(JSON.stringify(result===false?{version:null}:{version:result.nextRelease.version,notes:result.nextRelease.notes}));`;
  return JSON.parse(execFileSync(process.execPath,['--input-type=module','--eval',program,sourceRoot],{encoding:'utf8',env:environment}));
}

function fixture(message, initial = false) {
  const dir = mkdtempSync(join(tmpdir(),'multicad-release-contract-'));
  git(dir,['init','-b','main']);
  git(dir,['config','user.name','Release test']);git(dir,['config','user.email','test@example.invalid']);
  writeFileSync(join(dir,'Cargo.toml'),'[workspace.package]\nversion = "0.1.0-dev"\n');
  // A clone has no installed dependencies: preview must bypass its config.
  writeFileSync(join(dir,'release.config.cjs'),"require('multicad-deliberately-uninstalled-fixture');\n");
  git(dir,['add','Cargo.toml','release.config.cjs']);git(dir,['commit','-m','chore: initial fixture']);
  if(!initial)git(dir,['tag','v1.2.3']);
  git(dir,['commit','--allow-empty','-m',message]);
  return dir;
}

for(const [message,expected,initial] of [
  ['fix: repair a reference','1.2.4',false],
  ['perf: improve picking','1.2.4',false],
  ['feat: add a sketch','1.3.0',false],
  ['feat!: replace project format','2.0.0',false],
  ['ci: configure runners',null,false],
  ['feat: first native feature','0.1.0',true],
  ['fix: first reference repair','0.0.1',true],
])test(`actual release-engine preview: ${message}`,async()=>{
  const dir=fixture(message,initial);
  try{
    const before=git(dir,['show-ref']);
    const result=preview(dir);
    assert.equal(result.version,expected);
    assert.equal(git(dir,['show-ref']),before,'preview must not mutate source refs');
    assert.equal(git(dir,['status','--porcelain']),'');
    assert.equal(git(dir,['remote','-v']),'');
    if(result.version)assert.match(result.notes,/sketch|reference|picking|format|feature/i);
  }finally{rmSync(dir,{recursive:true,force:true});}
});

test('an already tagged commit is a no-op on retry',async()=>{
  const dir=fixture('fix: released change');
  try{
    git(dir,['tag','v1.2.4']);const refs=git(dir,['show-ref']);
    assert.equal(preview(dir).version,null);
    assert.equal(git(dir,['show-ref']),refs);
  }finally{rmSync(dir,{recursive:true,force:true});}
});

test('tag-only configuration has no npm or release-commit plugin',()=>{
  assert.deepEqual(config.branches,['main']);assert.equal(config.tagFormat,'v${version}');
  const plugins=config.plugins.map(p=>Array.isArray(p)?p[0]:p);
  assert(!plugins.some(p=>/semantic-release[/\\](?:git|npm)[/\\]/u.test(p)));
  const githubPlugin=require.resolve('@semantic-release/github');
  assert(plugins.includes(githubPlugin));
  const provider=config.plugins.find(p=>Array.isArray(p)&&p[0]===githubPlugin)[1];
  assert.equal(provider.successComment,false);assert.equal(provider.failComment,false);
});

test('publication is disabled unless every trusted activation condition is present',()=>{
  const env={GITHUB_ACTIONS:'true',GITHUB_EVENT_NAME:'push',GITHUB_REF:'refs/heads/main',GITHUB_REPOSITORY:'example/application',GITHUB_SHA:'a'.repeat(40),MULTICAD_EXPECTED_RELEASE_VERSION:'1.2.3',MULTICAD_RELEASES_ENABLED:'true',GITHUB_TOKEN:'test-only'};
  assert.doesNotThrow(()=>requirePublicationEnvironment(env));
  for(const key of Object.keys(env)){const copy={...env};delete copy[key];assert.throws(()=>requirePublicationEnvironment(copy),undefined,key);}
  assert.throws(()=>requirePublicationEnvironment({...env,GITHUB_EVENT_NAME:'pull_request'}));
  assert.throws(()=>requirePublicationEnvironment({...env,GITHUB_REF:'refs/tags/v1.2.3'}));
  assert.throws(()=>githubRepositoryUrl({...env,GITHUB_SERVER_URL:'https://forgejo.example.invalid'}));
});

test('version and repository inputs cannot escape their intended scope',()=>{
  for(const value of ['v1.2.3','1.2','1.2.3\nrelease=true','../1.2.3'])assert.throws(()=>requireVersion(value));
  for(const repository of ['owner/repo/extra','../repo','https://example.invalid/repo'])assert.throws(()=>githubRepositoryUrl({GITHUB_REPOSITORY:repository}));
  assert.equal(githubRepositoryUrl({GITHUB_REPOSITORY:'owner/repo'}),'https://github.com/owner/repo.git');
});

test('native regression suites are read-only and gate publication on every platform',()=>{
  const regression=readFileSync(new URL('../.github/workflows/native-regression.yml',import.meta.url),'utf8');
  const release=readFileSync(new URL('../.github/workflows/release.yml',import.meta.url),'utf8');
  assert.match(regression,/^permissions:\n {2}contents: read$/m);
  assert.doesNotMatch(regression,/secrets\.|: *write|npm publish|semantic-release|git push|git tag/);
  for(const os of ['ubuntu-24\\.04','macos-15','windows-2025'])assert.match(regression,new RegExp(`- os: ${os}\\n`));
  for(const suite of ['--bin multicad files::','--test release_hardening','--test project_file','--test drawing_exports','--test control_desk','cargo test --workspace --locked --no-fail-fast'])assert(regression.includes(suite),suite);
  assert.match(release,/regression:\n {4}name: Native regression suites\n {4}uses: \.\/\.github\/workflows\/native-regression\.yml/);
  assert.match(release,/needs: \[metadata, quality, build, foundation, regression\]/);
});
