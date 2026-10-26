import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { copyFileSync, existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, renameSync, rmSync, writeFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { requireCommit, requireVersion } from './release-policy.mjs';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
export const targets = Object.freeze(['linux-x64', 'macos-arm64', 'windows-x64']);
export const outputDirectory = join(root, 'artifacts/release');
const run = (command, args, options = {}) => execFileSync(command, args, {
  cwd: root, encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'], ...options,
}).trim();
export const digest = (path) => createHash('sha256').update(readFileSync(path)).digest('hex');
export const archiveName = (version, target) => `multicad-bootstrap-${requireVersion(version)}-${target}.zip`;

export function validateDescriptor(value, version, commit, target, archiveHash) {
  requireVersion(version); requireCommit(commit);
  if (!targets.includes(target) || value.version !== version || value.tag !== `v${version}` || value.commit !== commit ||
      value.target !== target || value.sha256 !== archiveHash || value.archive !== archiveName(version, target) ||
      value.kind !== 'diagnostic-bootstrap') {
    throw new Error(`Mismatched release asset identity for ${target}`);
  }
}

export function verifyPackageSet(directory, version, commit) {
  requireVersion(version); requireCommit(commit);
  const allowed = new Set(['release-manifest.json']);
  const assets = targets.map((target) => {
    const name = archiveName(version, target);
    for (const suffix of ['', '.json', '.sha256']) allowed.add(name + suffix);
    const archive = join(directory, name);
    if (!existsSync(archive)) throw new Error(`Missing required ${target} archive`);
    const hash = digest(archive);
    const descriptor = JSON.parse(readFileSync(archive + '.json', 'utf8'));
    validateDescriptor(descriptor, version, commit, target, hash);
    if (readFileSync(archive + '.sha256', 'utf8') !== `${hash}  ${name}\n`) {
      throw new Error(`Checksum manifest mismatch for ${target}`);
    }
    const magic = readFileSync(archive).subarray(0, 4).toString('hex');
    if (magic !== '504b0304') throw new Error(`Not a ZIP archive: ${name}`);
    return descriptor;
  });
  const extras = readdirSync(directory).filter((name) => !allowed.has(name));
  if (extras.length) throw new Error(`Unexpected/stale release files: ${extras.join(', ')}`);
  return { version, tag: `v${version}`, commit, kind: 'diagnostic-bootstrap', assets };
}

function buildPackage(version) {
  requireVersion(version);
  if (run('git', ['status', '--porcelain', '--untracked-files=no'])) {
    throw new Error('Commit tracked changes before packaging a source-identified release');
  }
  const commit = requireCommit(run('git', ['rev-parse', 'HEAD']));
  const target = `${{ darwin: 'macos', win32: 'windows', linux: 'linux' }[process.platform]}-${process.arch}`;
  if (!targets.includes(target)) throw new Error(`Unsupported packaging host: ${target}`);
  const env = { ...process.env, MULTICAD_RELEASE_VERSION: version, MULTICAD_BUILD_COMMIT: commit };
  run('cargo', ['test', '--workspace', '--locked'], { env, stdio: 'inherit' });
  run('cargo', ['build', '--workspace', '--release', '--locked'], { env, stdio: 'inherit' });
  const metadata = JSON.parse(run('cargo', ['metadata', '--no-deps', '--format-version', '1', '--locked']));
  const binaryName = process.platform === 'win32' ? 'multicad.exe' : 'multicad';
  const binary = join(metadata.target_directory, 'release', binaryName);
  if (run(binary, ['--version']) !== `MultiCAD ${version}`) throw new Error('Compiled version does not match package');
  const diagnostics = run(binary, ['--diagnostics']);
  const rustOS = { darwin: 'macos', win32: 'windows', linux: 'linux' }[process.platform];
  const rustArch = { x64: 'x86_64', arm64: 'aarch64' }[process.arch];
  if (!diagnostics.includes(`Source commit: ${commit}`) || !diagnostics.includes(`Target: ${rustOS}/${rustArch}`)) {
    throw new Error('Compiled identity/target does not match this source and host');
  }
  mkdirSync(outputDirectory, { recursive: true });
  mkdirSync(join(root, 'artifacts/staging'), { recursive: true });
  const scratch = mkdtempSync(join(root, 'artifacts/staging/package-'));
  try {
    const stem = archiveName(version, target).slice(0, -4);
    const stage = join(scratch, stem);
    mkdirSync(stage);
    copyFileSync(binary, join(stage, binaryName));
    copyFileSync(join(root, 'LICENSE'), join(stage, 'LICENSE'));
    copyFileSync(join(root, 'docs/BUILDING.md'), join(stage, 'BUILDING.md'));
    writeFileSync(join(stage, 'BOOTSTRAP.txt'), `${diagnostics}\n\nDiagnostic bootstrap only; not a CAD desktop installer.\n`);
    const archive = join(scratch, stem + '.zip');
    if (process.platform === 'darwin') run('ditto', ['-c', '-k', '--keepParent', stage, archive]);
    else if (process.platform === 'linux') run('zip', ['-q', '-r', archive, stem], { cwd: scratch });
    else run('powershell.exe', ['-NoProfile', '-NonInteractive', '-Command',
      "$ErrorActionPreference='Stop'; Compress-Archive -LiteralPath $env:MULTICAD_STAGE -DestinationPath $env:MULTICAD_ZIP -CompressionLevel Optimal"],
      { env: { ...env, MULTICAD_STAGE: stage, MULTICAD_ZIP: archive } });
    const name = archiveName(version, target);
    const hash = digest(archive);
    const descriptor = { version, tag: `v${version}`, commit, target, archive: name, sha256: hash, kind: 'diagnostic-bootstrap' };
    writeFileSync(join(scratch, name + '.json'), JSON.stringify(descriptor, null, 2) + '\n');
    writeFileSync(join(scratch, name + '.sha256'), `${hash}  ${name}\n`);
    for (const suffix of ['', '.json', '.sha256']) renameSync(join(scratch, name + suffix), join(outputDirectory, name + suffix));
    if (run('git', ['status', '--porcelain', '--untracked-files=no'])) throw new Error('Packaging modified tracked files');
    console.log(JSON.stringify(descriptor, null, 2));
  } finally {
    rmSync(scratch, { recursive: true, force: true });
  }
}

if (process.argv[1] && pathToFileURL(resolve(process.argv[1])).href === import.meta.url) {
  try {
    const [action, version, commit, ...extra] = process.argv.slice(2);
    if (extra.length) throw new Error('Unexpected arguments');
    if (action === 'build' && version && !commit) buildPackage(version);
    else if (action === 'verify' && version && commit) {
      const manifest = verifyPackageSet(outputDirectory, version, commit);
      writeFileSync(join(outputDirectory, 'release-manifest.json'), JSON.stringify(manifest, null, 2) + '\n');
      console.log(`Verified ${manifest.assets.length} archives for v${version} at ${commit}`);
    } else throw new Error('Usage: package-release.mjs build VERSION | verify VERSION COMMIT');
  } catch (error) { console.error(error.message); process.exitCode = 1; }
}
