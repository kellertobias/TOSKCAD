const { appendFileSync } = require("node:fs");
const { execFileSync } = require("node:child_process");

async function verifyRelease(_configuration, context) {
  const { nextRelease, cwd, env } = context;
  const expected = env.MULTICAD_EXPECTED_RELEASE_VERSION;
  if (expected && expected !== nextRelease.version) {
    throw new Error(`Version changed after the build: expected ${expected}, selected ${nextRelease.version}`);
  }
  if (env.MULTICAD_PUBLISHING === "true") {
    if (nextRelease.gitHead !== env.GITHUB_SHA) throw new Error("Release is not on the tested commit");
    const response = await fetch(`https://api.github.com/repos/${env.GITHUB_REPOSITORY}/git/ref/heads/main`, {
      headers: { authorization: `Bearer ${env.GITHUB_TOKEN || env.GH_TOKEN}`, accept: "application/vnd.github+json" },
    });
    if (!response.ok || (await response.json()).object.sha !== env.GITHUB_SHA) {
      throw new Error("Refusing to tag a superseded or unverifiable main commit");
    }
  }
  // Only a disposable preview clone or the ephemeral release checkout may have
  // this bootstrap tag. It must never be pushed as a real release.
  if (env.MULTICAD_BOOTSTRAP_TAG) {
    execFileSync("git", ["tag", "--delete", env.MULTICAD_BOOTSTRAP_TAG], { cwd, stdio: "ignore" });
    delete env.MULTICAD_BOOTSTRAP_TAG;
  }
  if (env.MULTICAD_RELEASE_OUTPUT) {
    appendFileSync(env.MULTICAD_RELEASE_OUTPUT, `version=${nextRelease.version}\nrelease=true\n`);
  }
}

module.exports = { verifyRelease };
