import semver from "semver";

export function requireVersion(value) {
  if (typeof value !== "string" || semver.valid(value) !== value) {
    throw new Error("A canonical SemVer version without a v prefix is required");
  }
  return value;
}

export function requireCommit(value) {
  if (!/^[a-f0-9]{40}$/u.test(value || "")) {
    throw new Error("A full Git commit SHA is required");
  }
  return value;
}

export function githubRepositoryUrl(environment = process.env) {
  const repository = environment.GITHUB_REPOSITORY;
  if (!/^[A-Za-z0-9][A-Za-z0-9-]*\/[A-Za-z0-9_.-]+$/u.test(repository || "") ||
      /^\.+$/u.test(repository?.split("/")[1] || "")) {
    throw new Error("GITHUB_REPOSITORY must identify the GitHub repository");
  }
  if (environment.GITHUB_SERVER_URL && environment.GITHUB_SERVER_URL !== "https://github.com") {
    throw new Error("This release adapter targets github.com");
  }
  return `https://github.com/${repository}.git`;
}

export function requirePublicationEnvironment(environment = process.env) {
  if (
    environment.GITHUB_ACTIONS !== "true" ||
    environment.GITHUB_EVENT_NAME !== "push" ||
    environment.GITHUB_REF !== "refs/heads/main" ||
    environment.MULTICAD_RELEASES_ENABLED !== "true"
  ) {
    throw new Error("Publication requires an enabled, trusted GitHub main push");
  }
  githubRepositoryUrl(environment);
  requireCommit(environment.GITHUB_SHA);
  requireVersion(environment.MULTICAD_EXPECTED_RELEASE_VERSION);
  if (!environment.GITHUB_TOKEN && !environment.GH_TOKEN) {
    throw new Error("Publication requires the ephemeral GitHub repository token");
  }
}

export function nextVersionFromResult(result) {
  if (result === false) return null;
  return requireVersion(result.nextRelease.version);
}
