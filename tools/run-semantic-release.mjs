import { execFileSync } from "node:child_process";
import { createRequire } from "node:module";
import { appendFileSync, existsSync, mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import semanticRelease from "semantic-release";
import semver from "semver";
import { outputDirectory, verifyPackageSet } from "./package-release.mjs";
import {
  githubRepositoryUrl, requireCommit, requirePublicationEnvironment, requireVersion,
} from "./release-policy.mjs";

const require = createRequire(import.meta.url);
const configuration = require("../release.config.cjs");
const { previewPlugins } = require("./semantic-release-plugins.cjs");
const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const git = (args, cwd = root) => execFileSync("git", args, {
  cwd, encoding: "utf8", stdio: ["ignore", "pipe", "pipe"],
}).trim();

export function addBootstrapTag(cwd, environment) {
  const tags = git(["tag", "--merged", "HEAD"], cwd).split("\n");
  if (tags.some((tag) => tag.startsWith("v") && semver.valid(tag.slice(1)))) return null;
  const tag = "v0.0.0";
  if (git(["tag", "--list", tag], cwd)) throw new Error("Bootstrap tag already exists off the current history");
  const first = git(["rev-list", "--max-parents=0", "HEAD"], cwd).split("\n")[0];
  git(["tag", tag, first], cwd);
  environment.MULTICAD_BOOTSTRAP_TAG = tag;
  return tag;
}

async function requireLatestMain(environment) {
  const response = await fetch(
    `https://api.github.com/repos/${environment.GITHUB_REPOSITORY}/git/ref/heads/main`,
    { headers: { authorization: `Bearer ${environment.GITHUB_TOKEN || environment.GH_TOKEN}`, accept: "application/vnd.github+json" } },
  );
  if (!response.ok) throw new Error(`Could not verify GitHub main (${response.status})`);
  const ref = await response.json();
  if (ref.object.sha !== environment.GITHUB_SHA) throw new Error("Refusing to release a superseded main commit");
}

export function buildVersion(sourceRoot=root) {
  const versions=git(["tag","--merged","HEAD"],sourceRoot).split("\n").filter(t=>t.startsWith("v")&&semver.valid(t.slice(1))).map(t=>t.slice(1));
  const workspace=readFileSync(join(sourceRoot,"Cargo.toml"),"utf8").match(/\[workspace\.package\]([\s\S]*?)(?:\n\[|$)/u)?.[1];
  return requireVersion(semver.rsort(versions)[0]||workspace?.match(/^version = "([^"]+)"/mu)?.[1]);
}

export async function runRelease({ dryRun = false, sourceRoot = root, stdout, stderr } = {}) {
  sourceRoot = resolve(sourceRoot);
  if (!dryRun && sourceRoot !== root) throw new Error("Publication must use this repository checkout");
  let scratch;
  let bootstrap;
  let cwd = sourceRoot;
  const environment = { ...process.env };
  const options = { ...configuration, ci: !dryRun };
  if (environment.MULTICAD_RELEASE_OUTPUT) {
    appendFileSync(environment.MULTICAD_RELEASE_OUTPUT, `version=${buildVersion(sourceRoot)}\nrelease=false\n`);
  }
  try {
    if (dryRun) {
      // Clone locally, never push or modify the source checkout's refs/index.
      scratch = mkdtempSync(join(tmpdir(), "multicad-release-preview-"));
      const remote = join(scratch, "remote.git");
      cwd = join(scratch, "work");
      const commit = git(["rev-parse", "HEAD"], sourceRoot);
      git(["clone", "--quiet", "--no-hardlinks", "--bare", sourceRoot, remote]);
      git(["--git-dir", remote, "symbolic-ref", "HEAD", "refs/heads/main"]);
      git(["--git-dir", remote, "update-ref", "refs/heads/main", commit]);
      git(["clone", "--quiet", "--no-hardlinks", "--branch", "main", remote, cwd]);
      // The cloned config resolves dependencies relative to the clone, which
      // intentionally has no node_modules. Use the explicit source-resolved
      // options below; prevent cosmiconfig from loading that cloned module.
      rmSync(join(cwd, "release.config.cjs"), { force: true });
      options.repositoryUrl = pathToFileURL(remote).href;
      options.plugins = previewPlugins;
      options.dryRun = true;
      delete environment.GITHUB_TOKEN;
      delete environment.GH_TOKEN;
      delete environment.MULTICAD_EXPECTED_RELEASE_VERSION;
      delete environment.MULTICAD_PUBLISHING;
    } else {
      requirePublicationEnvironment(environment);
      if (git(["status", "--porcelain", "--untracked-files=no"])) throw new Error("Tracked files must be clean");
      if (git(["rev-parse", "HEAD"]) !== requireCommit(environment.GITHUB_SHA)) throw new Error("Checkout is not the tested commit");
      const manifest = verifyPackageSet(outputDirectory, environment.MULTICAD_EXPECTED_RELEASE_VERSION, environment.GITHUB_SHA);
      const recorded = JSON.parse(readFileSync(join(outputDirectory, "release-manifest.json"), "utf8"));
      if (JSON.stringify(recorded) !== JSON.stringify(manifest)) throw new Error("Release manifest is stale or mismatched");
      await requireLatestMain(environment);
      environment.MULTICAD_PUBLISHING = "true";
      options.repositoryUrl = githubRepositoryUrl(environment);
    }
    bootstrap = addBootstrapTag(cwd, environment);
    const result = await semanticRelease(options, { cwd, env: environment, ...(stdout ? { stdout } : {}), ...(stderr ? { stderr } : {}) });
    if (result === false && environment.MULTICAD_RELEASE_OUTPUT) {
      appendFileSync(environment.MULTICAD_RELEASE_OUTPUT, "release=false\n");
    }
    if (!dryRun && git(["status", "--porcelain", "--untracked-files=no"])) {
      throw new Error("Release changed tracked files; branch commits are forbidden");
    }
    return result;
  } finally {
    if (bootstrap && existsSync(join(cwd, ".git")) && git(["tag", "--list", bootstrap], cwd)) git(["tag", "--delete", bootstrap], cwd);
    if (scratch) rmSync(scratch, { recursive: true, force: true });
  }
}

if (process.argv[1] && pathToFileURL(resolve(process.argv[1])).href === import.meta.url) {
  const args = process.argv.slice(2);
  if (args.length && (args.length !== 1 || args[0] !== "--dry-run")) {
    console.error("Only --dry-run is supported"); process.exitCode = 1;
  } else runRelease({ dryRun: args[0] === "--dry-run" }).catch((error) => { console.error(error.message); process.exitCode = 1; });
}
